//! AGENT-007 to AGENT-011 and the framework half of AGENT-017: agent
//! accounts under auth.md v0.6.0, the Magnetar agent auth plugin wired into
//! the framework beside the OAuth server, driven through `handle_request`
//! the way an agent, a provider and the user's browser would drive it.
//!
//! The user's sessions are the framework's own session guard's here;
//! `agent_auth_magnetar` covers Magnetar-owned sessions. Every body and
//! error code of auth.md is checked at the Magnetar level by lane C's
//! suite; these tests check that the framework serves them unchanged and
//! adds what the host owes: the claim page, the resolver, the user and
//! operator surfaces, the audit trail and the build checks.

use std::any::Any;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use sea_orm::{ConnectionTrait, DatabaseConnection};
use secrecy::ExposeSecret;
use serde_json::{Value, json};
use suprnova::agents::{
    AgentAuth, AgentClaimed, AgentUserProvisioner, AgentsConfig, CallLimit, ClientMetadataFetcher,
    ClientMetadataPolicy, IdentityType, OAuthServer, ProvidedIdentity, TrustedIssuer,
};
use suprnova::events::testing::dispatched;
use suprnova::magnetar_integration::engine::MagnetarLifecycleEvent;
use suprnova::testing::{TestClient, TestResponse};
use suprnova::{EventFacade, FrameworkError, Router};

use crate::fixture::scope;
use crate::oauth::{
    FakeDocuments, FakeResolver, account_email, container, form_token, location, mark_unverified,
    new_account, path_of, query_param, registry, remember_account, sign_in,
};
use crate::outside_issuer::IssuerKey;

/// The provider whose ID-JAGs the application trusts.
pub const PROVIDER: &str = "https://agents.provider.test";
pub const PROVIDER_HOST: &str = "agents.provider.test";
pub const PROVIDER_JWKS: &str = "https://agents.provider.test/.well-known/jwks.json";
/// The name the application's trust list gives the provider.
pub const PROVIDER_NAME: &str = "Acme Agents";
/// The authorization server's issuer under the test's `APP_URL`.
pub const ISSUER: &str = "http://localhost";
/// The MCP endpoint's canonical URL under the test's `APP_URL`.
pub const RESOURCE: &str = "http://localhost/mcp";
/// The address every request of a world comes from.
pub const PEER: &str = "203.0.113.9";
const ID_JAG_TYPE: &str = "urn:ietf:params:oauth:token-type:id-jag";
const JWT_BEARER: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";
const CLAIM_GRANT: &str = "urn:workos:agent-auth:grant-type:claim";
const REVOKED_EVENT: &str =
    "https://schemas.workos.com/events/agent/auth/identity/assertion/revoked";

// ------------------------------------------------------------- world ----

/// A fresh P-256 service key in PKCS#8 PEM.
pub fn service_pem() -> String {
    magnetar::agent_auth::SigningKey::generate_pem()
        .expect("a P-256 key")
        .expose_secret()
        .to_owned()
}

/// The authorization server's and agent auth's tables on a fresh in-memory
/// database.
pub async fn agent_database() -> DatabaseConnection {
    let db = crate::oauth::database().await;
    AgentAuth::migrate(&db).await.expect("agent auth's tables");
    db
}

/// One application with the OAuth server and agent auth on, and a
/// browser-like client at [`PEER`].
pub struct AgentWorld {
    pub client: TestClient,
    pub db: DatabaseConnection,
    pub resolver: Arc<FakeResolver>,
    pub documents: Arc<FakeDocuments>,
    pub provider: IssuerKey,
    pub pem: String,
    _container: Box<dyn Any>,
}

/// The agent auth every world starts from: the service key `pem` as
/// `service-1`, key sets through the fakes, `posts:read` before a claim
/// and `posts:read` and `posts:write` after.
pub fn base_agent_auth(
    pem: &str,
    resolver: &Arc<FakeResolver>,
    documents: &Arc<FakeDocuments>,
) -> AgentAuth {
    AgentAuth::new()
        .signing_key("service-1", pem)
        .key_set_fetch(resolver.clone(), documents.clone())
        .pre_claim_scopes([scope::POSTS_READ])
        .post_claim_scopes([scope::POSTS_READ, scope::POSTS_WRITE])
}

/// The router of one node: the fixture's routes, the OAuth server on `db`
/// and agent auth as `configure` adjusts it.
pub fn agent_router(
    db: &DatabaseConnection,
    pem: &str,
    resolver: &Arc<FakeResolver>,
    documents: &Arc<FakeDocuments>,
    configure: impl FnOnce(AgentAuth) -> AgentAuth,
) -> Router {
    let fetcher = Arc::new(ClientMetadataFetcher::new(
        resolver.clone(),
        documents.clone(),
        ClientMetadataPolicy::default(),
    ));
    let agent = configure(base_agent_auth(pem, resolver, documents));
    crate::oauth::router(
        AgentsConfig::new()
            .oauth_server(
                OAuthServer::new()
                    .connection(db.clone())
                    .client_metadata_fetcher(fetcher),
            )
            .agent_auth(agent),
    )
}

/// A world whose agent auth `configure` adjusts.
pub async fn agent_world(configure: impl FnOnce(AgentAuth) -> AgentAuth) -> AgentWorld {
    let db = agent_database().await;
    agent_world_on(db, configure)
}

/// A world on `db`.
pub fn agent_world_on(
    db: DatabaseConnection,
    configure: impl FnOnce(AgentAuth) -> AgentAuth,
) -> AgentWorld {
    agent_world_in(container(), db, configure)
}

/// A world on `db` in the per-test `container`.
pub fn agent_world_in(
    container: Box<dyn Any>,
    db: DatabaseConnection,
    configure: impl FnOnce(AgentAuth) -> AgentAuth,
) -> AgentWorld {
    let resolver = Arc::new(FakeResolver::default());
    resolver.answer(PROVIDER_HOST, "93.184.215.14");
    let documents = Arc::new(FakeDocuments::default());
    let provider = IssuerKey::generate("provider-1");
    documents.serve(PROVIDER_JWKS, &json!({ "keys": [provider.jwk()] }));
    let pem = service_pem();
    let client = TestClient::new(
        agent_router(&db, &pem, &resolver, &documents, configure),
        registry(),
    )
    .with_peer_address(PEER.parse().unwrap());
    AgentWorld {
        client,
        db,
        resolver,
        documents,
        provider,
        pem,
        _container: container,
    }
}

impl AgentWorld {
    /// A second node: its own router, authorization server and engine, on
    /// the same database, with the same key unless `configure` names
    /// another.
    pub fn node(&self, configure: impl FnOnce(AgentAuth) -> AgentAuth) -> TestClient {
        TestClient::new(
            agent_router(
                &self.db,
                &self.pem,
                &self.resolver,
                &self.documents,
                configure,
            ),
            registry(),
        )
        .with_peer_address(PEER.parse().unwrap())
    }

    /// A second node behind `registry` instead of the session and CSRF
    /// middleware.
    pub fn node_with(
        &self,
        registry: suprnova::MiddlewareRegistry,
        configure: impl FnOnce(AgentAuth) -> AgentAuth,
    ) -> TestClient {
        TestClient::new(
            agent_router(
                &self.db,
                &self.pem,
                &self.resolver,
                &self.documents,
                configure,
            ),
            registry,
        )
        .with_peer_address(PEER.parse().unwrap())
    }

    /// `claims` signed by the provider as an ID-JAG.
    pub fn id_jag(&self, claims: &Value) -> String {
        self.provider
            .sign_as(&self.provider.kid, Some("oauth-id-jag+jwt"), claims)
    }

    /// `claims` signed by the provider as a Security Event Token.
    pub fn event(&self, claims: &Value) -> String {
        self.provider
            .sign_as(&self.provider.kid, Some("secevent+jwt"), claims)
    }
}

/// Agent auth that also accepts the test provider's ID-JAGs.
pub fn with_id_jags(agent: AgentAuth) -> AgentAuth {
    agent
        .enable(IdentityType::IdentityAssertion)
        .trust(TrustedIssuer::new(PROVIDER, PROVIDER_JWKS, PROVIDER_NAME))
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn unique(label: &str) -> String {
    format!("{label}-{}", uuid::Uuid::new_v4().simple())
}

/// The claims of a valid ID-JAG for `subject` with `email` verified.
pub fn id_jag_claims(subject: &str, email: &str) -> Value {
    json!({
        "iss": PROVIDER,
        "sub": subject,
        "aud": ISSUER,
        "client_id": PROVIDER,
        "jti": unique("jti"),
        "iat": now(),
        "exp": now() + 300,
        "auth_time": now() - 60,
        "email": email,
        "email_verified": true,
        "name": "Provider-Chosen Name",
        "agent_platform": "test-platform",
        "agent_context_id": "chat-1",
    })
}

/// A Security Event Token's claims revoking `subject`'s delegation.
fn revocation_event(subject: &str) -> Value {
    json!({
        "iss": PROVIDER,
        "sub": subject,
        "aud": ISSUER,
        "jti": unique("set"),
        "iat": now(),
        "events": { REVOKED_EVENT: {} },
    })
}

fn with(mut claims: Value, field: &str, value: Value) -> Value {
    claims[field] = value;
    claims
}

fn without(mut claims: Value, field: &str) -> Value {
    claims.as_object_mut().unwrap().remove(field);
    claims
}

/// The decoded payload of a JWT.
fn jwt_part(token: &str, index: usize) -> Value {
    let part = token.split('.').nth(index).expect("a JWT part");
    serde_json::from_slice(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(part)
            .expect("base64url"),
    )
    .expect("JSON")
}

// ------------------------------------------------------------ steps ----

async fn register(client: &TestClient, body: Value) -> TestResponse {
    client.post("/agent/identity").json(&body).send().await
}

/// Register anonymously and answer the body.
async fn anonymous(client: &TestClient) -> Value {
    let response = register(client, json!({ "type": "anonymous" })).await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    response.json()
}

/// Register by `service_auth` for `email` and answer the body.
async fn service_auth(client: &TestClient, email: &str) -> Value {
    let response = register(
        client,
        json!({ "type": "service_auth", "login_hint": email }),
    )
    .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    response.json()
}

async fn present(client: &TestClient, id_jag: &str) -> TestResponse {
    register(
        client,
        json!({
            "type": "identity_assertion",
            "assertion_type": ID_JAG_TYPE,
            "assertion": id_jag,
        }),
    )
    .await
}

/// Exchange an identity assertion at the token endpoint.
async fn exchange(client: &TestClient, assertion: &str, scope: Option<&str>) -> TestResponse {
    let mut form = vec![
        ("grant_type", JWT_BEARER),
        ("assertion", assertion),
        ("resource", RESOURCE),
    ];
    if let Some(scope) = scope {
        form.push(("scope", scope));
    }
    client.post("/oauth/token").form(&form).send().await
}

/// The access token an assertion exchanges for.
async fn agent_token(client: &TestClient, assertion: &str) -> String {
    let tokens = exchange(client, assertion, None).await;
    assert_eq!(tokens.status(), 200, "{}", tokens.body_text());
    tokens.json()["access_token"].as_str().unwrap().to_owned()
}

async fn initiate(client: &TestClient, claim_token: &str, email: &str) -> TestResponse {
    client
        .post("/agent/identity/claim")
        .json(&json!({ "claim_token": claim_token, "email": email }))
        .send()
        .await
}

async fn poll(client: &TestClient, claim_token: &str) -> TestResponse {
    client
        .post("/oauth/token")
        .form(&[
            ("grant_type", CLAIM_GRANT),
            ("claim_token", claim_token),
            ("resource", RESOURCE),
        ])
        .send()
        .await
}

/// The value of a hidden form field on a page.
fn hidden(page: &str, name: &str) -> String {
    let marker = format!("name=\"{name}\" value=\"");
    let start = page
        .find(&marker)
        .unwrap_or_else(|| panic!("a hidden {name} in {page}"))
        + marker.len();
    let end = page[start..].find('"').unwrap() + start;
    page[start..end].to_owned()
}

/// Open the verification URI in the browser.
async fn open_claim(client: &TestClient, verification_uri: &str) -> TestResponse {
    client.get(path_of(verification_uri)).send().await
}

/// Post the claim page's form with `code`.
async fn confirm(client: &TestClient, page: &TestResponse, code: &str) -> TestResponse {
    let text = page.body_text();
    client
        .post("/agent/claim")
        .form(&[
            ("_token", form_token(&text).as_str()),
            (
                "claim_attempt_token",
                hidden(&text, "claim_attempt_token").as_str(),
            ),
            ("user_code", code),
        ])
        .send()
        .await
}

/// Forget every poll, so the next one is not answered `slow_down`.
async fn forget_polls(db: &DatabaseConnection) {
    db.execute_unprepared("UPDATE auth_agent_registrations SET last_polled_at = NULL")
        .await
        .unwrap();
}

/// What a claimed anonymous registration left: its registration body,
/// the claim token and the poll's tokens.
struct Claimed {
    registration: Value,
    tokens: Value,
}

/// Register anonymously, have `user` (signed in on `client`) claim it, and
/// poll for the tokens.
async fn claim_anonymous(world: &AgentWorld, user: &str) -> Claimed {
    claim_anonymous_on(&world.client, &world.db, user).await
}

/// [`claim_anonymous`] through `client`, a node on `db`.
async fn claim_anonymous_on(client: &TestClient, db: &DatabaseConnection, user: &str) -> Claimed {
    let registration = anonymous(client).await;
    let claim_token = registration["claim_token"].as_str().unwrap();
    let started = initiate(client, claim_token, &account_email(user)).await;
    assert_eq!(started.status(), 200, "{}", started.body_text());
    let attempt = started.json()["claim_attempt"].clone();
    sign_in(client, user).await;
    let page = open_claim(client, attempt["verification_uri"].as_str().unwrap()).await;
    assert_eq!(page.status(), 200, "{}", page.body_text());
    let done = confirm(client, &page, attempt["user_code"].as_str().unwrap()).await;
    assert_eq!(done.status(), 200, "{}", done.body_text());
    forget_polls(db).await;
    let tokens = poll(client, claim_token).await;
    assert_eq!(tokens.status(), 200, "{}", tokens.body_text());
    Claimed {
        registration,
        tokens: tokens.json(),
    }
}

async fn call(client: &TestClient, token: &str, tool: &str, arguments: Value) -> (u16, Value) {
    let result = client.mcp().with_token(token).call(tool, arguments).await;
    (
        result.status(),
        result.structured().cloned().unwrap_or(Value::Null),
    )
}

/// The `error_description` a refused token's 401 carries.
async fn refusal(client: &TestClient, token: &str) -> String {
    let refused = client
        .mcp()
        .with_token(token)
        .call("posts.show", json!({ "post": "1" }))
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

// ---------------------------------------------------------- AGENT-007 ----

#[tokio::test]
async fn agent_007_anonymous_agent_registers_and_calls_an_agent_tool() {
    let world = agent_world(|agent| agent).await;
    let client = &world.client;
    let registration = anonymous(client).await;
    assert_eq!(registration["registration_type"], "anonymous");
    assert_eq!(registration["pre_claim_scopes"], json!(["posts:read"]));
    assert_eq!(
        registration["post_claim_scopes"],
        json!(["posts:read", "posts:write"])
    );
    assert_eq!(
        registration["claim_url"],
        "http://localhost/agent/identity/claim"
    );
    let assertion = registration["identity_assertion"].as_str().unwrap();
    let header = jwt_part(assertion, 0);
    assert_eq!(header["typ"], "oauth-id-jag+jwt");
    assert_eq!(header["kid"], "service-1");
    let claims = jwt_part(assertion, 1);
    assert_eq!(claims["sub"], registration["registration_id"]);
    assert_eq!(claims["iss"], ISSUER);

    let token = agent_token(client, assertion).await;
    let (status, shown) = call(client, &token, "posts.show", json!({ "post": "7" })).await;
    assert_eq!(status, 200, "{shown}");
    assert_eq!(shown["post"], "7");
}

#[tokio::test]
async fn agent_007_service_auth_agent_claims_and_calls_an_agent_tool() {
    let world = agent_world(|agent| agent).await;
    let client = &world.client;
    let user = new_account("service-auth");
    let registration = service_auth(client, &account_email(&user)).await;
    assert_eq!(registration["registration_type"], "service_auth");
    assert!(registration.get("identity_assertion").is_none());
    let claim = &registration["claim"];
    let uri = claim["verification_uri"].as_str().unwrap();
    assert!(
        uri.starts_with("http://localhost/agent/claim?claim_attempt_token=cat_"),
        "the user opens the claim page: {uri}"
    );
    sign_in(client, &user).await;
    let page = open_claim(client, uri).await;
    assert_eq!(page.status(), 200, "{}", page.body_text());
    let done = confirm(client, &page, claim["user_code"].as_str().unwrap()).await;
    assert_eq!(done.status(), 200, "{}", done.body_text());
    assert!(done.body_text().contains("The agent is connected"));

    let tokens = poll(client, registration["claim_token"].as_str().unwrap()).await;
    assert_eq!(tokens.status(), 200, "{}", tokens.body_text());
    let tokens = tokens.json();
    let token = tokens["access_token"].as_str().unwrap();
    let (status, created) = call(
        client,
        token,
        "posts.create",
        json!({ "title": "From the agent" }),
    )
    .await;
    assert_eq!(status, 200, "{created}");
    assert_eq!(created["kind"], "agent");
    assert_eq!(created["user"], user);
    assert_eq!(created["auth_user"], user);
    assert_eq!(created["registration"], registration["registration_id"]);
    assert_eq!(created["claimed"], true);
}

/// A provisioner that creates the account an ID-JAG's email names.
struct CreateAccounts;

#[async_trait]
impl AgentUserProvisioner for CreateAccounts {
    async fn provision(
        &self,
        identity: &ProvidedIdentity,
    ) -> Result<Option<String>, FrameworkError> {
        let Some(id) = identity
            .verified_email
            .as_deref()
            .and_then(|email| email.strip_suffix("@accounts.test"))
        else {
            return Ok(None);
        };
        remember_account(id);
        Ok(Some(id.to_owned()))
    }
}

#[tokio::test]
async fn agent_007_identity_assertion_agent_registers_and_calls_an_agent_tool() {
    let world =
        agent_world(|agent| with_id_jags(agent).provision_users(Arc::new(CreateAccounts))).await;
    let client = &world.client;
    let user = unique("provisioned");
    let response = present(
        client,
        &world.id_jag(&id_jag_claims("provider-user-1", &account_email(&user))),
    )
    .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let registration = response.json();
    assert_eq!(registration["registration_type"], "identity_assertion");
    let assertion = registration["identity_assertion"].as_str().unwrap();
    assert_eq!(jwt_part(assertion, 1)["email"], account_email(&user));
    let token = agent_token(client, assertion).await;
    let (status, created) = call(client, &token, "posts.create", json!({ "title": "Hi" })).await;
    assert_eq!(status, 200, "{created}");
    assert_eq!(created["user"], user);
    assert_eq!(created["client"], PROVIDER, "the provider it came through");
}

/// The application's public root in the path prefix tests.
const PREFIXED: &str = "https://example.test/billing";

/// Name a public URL with a path prefix, before a world is built. Only in
/// a test with its own process: the configuration is process-wide.
fn serve_under_a_path_prefix() {
    suprnova::Config::register(
        suprnova::AppConfig::builder()
            .url(PREFIXED)
            .debug(false)
            .build(),
    );
}

#[tokio::test]
async fn agent_007_flow_under_a_path_prefix() {
    if crate::own_process_async::delegate(module_path!(), "agent_007_flow_under_a_path_prefix")
        .await
    {
        return;
    }
    serve_under_a_path_prefix();
    let world = agent_world(|agent| agent).await;
    let client = &world.client;
    let resource = format!("{PREFIXED}/mcp");

    let registration = anonymous(client).await;
    assert_eq!(
        registration["claim_url"],
        format!("{PREFIXED}/agent/identity/claim")
    );
    let assertion = registration["identity_assertion"].as_str().unwrap();
    assert_eq!(jwt_part(assertion, 1)["iss"], PREFIXED);
    let tokens = client
        .post("/oauth/token")
        .form(&[
            ("grant_type", JWT_BEARER),
            ("assertion", assertion),
            ("resource", resource.as_str()),
        ])
        .send()
        .await;
    assert_eq!(tokens.status(), 200, "{}", tokens.body_text());
    let token = tokens.json()["access_token"].as_str().unwrap().to_owned();
    let (status, shown) = call(client, &token, "posts.show", json!({ "post": "3" })).await;
    assert_eq!(status, 200, "{shown}");

    let metadata = client
        .get("/.well-known/oauth-authorization-server")
        .send()
        .await
        .json();
    assert_eq!(
        metadata["agent_auth"]["skill"],
        format!("{PREFIXED}/auth.md")
    );

    // The verification URI is the claim page under the prefix; a guest is
    // sent to sign in there and comes back to the page, its address
    // relative to the public root as the framework's redirects take one.
    let user = new_account("prefixed-claim");
    let registration = service_auth(client, &account_email(&user)).await;
    let uri = registration["claim"]["verification_uri"].as_str().unwrap();
    assert!(
        uri.starts_with(&format!("{PREFIXED}/agent/claim?claim_attempt_token=")),
        "{uri}"
    );
    let local = path_of(uri).replacen("/billing", "", 1);
    let guest = client.get(&local).send().await;
    let sign_in_at = location(&guest);
    assert!(
        sign_in_at.starts_with(&format!("{PREFIXED}/login?return_to=")),
        "{sign_in_at}"
    );
    assert_eq!(query_param(&sign_in_at, "return_to").unwrap(), local);
    sign_in(client, &user).await;
    let page = client.get(&local).send().await;
    assert_eq!(page.status(), 200, "{}", page.body_text());
    assert!(
        page.body_text()
            .contains(&format!("action=\"{PREFIXED}/agent/claim\"")),
        "{}",
        page.body_text()
    );
    let done = confirm(
        client,
        &page,
        registration["claim"]["user_code"].as_str().unwrap(),
    )
    .await;
    assert_eq!(done.status(), 200, "{}", done.body_text());
}

#[tokio::test]
async fn agent_011_auth_md_under_a_path_prefix() {
    if crate::own_process_async::delegate(module_path!(), "agent_011_auth_md_under_a_path_prefix")
        .await
    {
        return;
    }
    serve_under_a_path_prefix();
    let world = agent_world(with_id_jags).await;
    let text = world.client.get("/auth.md").send().await.body_text();
    for endpoint in [
        "/agent/identity",
        "/agent/identity/claim",
        "/agent/event/notify",
        "/oauth/token",
        "/oauth/revoke",
    ] {
        assert!(
            text.contains(&format!("{PREFIXED}{endpoint}")),
            "{endpoint} under the prefix in {text}"
        );
    }
    assert!(!text.contains("http://localhost"), "{text}");
}

#[tokio::test]
async fn agent_007_metadata_carries_the_agent_auth_block() {
    let world = agent_world(with_id_jags).await;
    for path in [
        "/.well-known/oauth-authorization-server",
        "/.well-known/openid-configuration",
    ] {
        let metadata = world.client.get(path).send().await;
        assert_eq!(metadata.status(), 200);
        let metadata = metadata.json();
        let block = &metadata["agent_auth"];
        assert_eq!(
            block["skill"], "http://localhost/auth.md",
            "{path}: {block}"
        );
        assert_eq!(
            block["identity_endpoint"], "http://localhost/agent/identity",
            "{block}"
        );
        let grants = metadata["grant_types_supported"].to_string();
        assert!(
            grants.contains(JWT_BEARER) && grants.contains(CLAIM_GRANT),
            "{grants}"
        );
    }
}

#[tokio::test]
async fn agent_007_identity_and_token_bodies_pass_through_unchanged() {
    let world = agent_world(|agent| agent).await;
    let client = &world.client;
    let response = register(client, json!({ "type": "anonymous" })).await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.header("cache-control"), Some("no-store"));
    let body = response.json();
    for field in [
        "registration_id",
        "registration_type",
        "identity_assertion",
        "assertion_expires",
        "pre_claim_scopes",
        "claim_url",
        "claim_token",
        "claim_token_expires",
        "post_claim_scopes",
    ] {
        assert!(body.get(field).is_some(), "{field} in {body}");
    }
    let refused = register(client, json!({ "type": "something" })).await;
    assert_eq!(refused.status(), 400);
    let refused = refused.json();
    assert_eq!(refused["error"], "invalid_request");
    assert_eq!(refused["error_description"], refused["message"]);
    let tokens = exchange(client, body["identity_assertion"].as_str().unwrap(), None).await;
    assert_eq!(tokens.header("cache-control"), Some("no-store"));
    let tokens = tokens.json();
    assert_eq!(tokens["token_type"], "Bearer");
    assert_eq!(tokens["scope"], "posts:read");
}

#[tokio::test]
async fn agent_007_token_endpoint_issues_no_refresh_token_for_agent_grants() {
    let world = agent_world(|agent| agent).await;
    let user = new_account("no-refresh");
    let claimed = claim_anonymous(&world, &user).await;
    assert!(
        claimed.tokens.get("refresh_token").is_none(),
        "{}",
        claimed.tokens
    );
    let assertion = claimed.registration["identity_assertion"].as_str().unwrap();
    // The pre-claim assertion stopped working with the claim; a fresh
    // anonymous one exchanges without a refresh token either.
    let fresh = anonymous(&world.client).await;
    let tokens = exchange(
        &world.client,
        fresh["identity_assertion"].as_str().unwrap(),
        None,
    )
    .await
    .json();
    assert!(tokens.get("refresh_token").is_none(), "{tokens}");
    assert_eq!(
        exchange(&world.client, assertion, None).await.json()["error"],
        "invalid_grant",
        "a pre-claim assertion stops at the claim"
    );
}

#[tokio::test]
async fn agent_007_disabled_type_answers_not_enabled() {
    let world = agent_world(|agent| {
        agent
            .disable(IdentityType::Anonymous)
            .disable(IdentityType::ServiceAuth)
            .enable(IdentityType::IdentityAssertion)
            .trust(TrustedIssuer::new(PROVIDER, PROVIDER_JWKS, PROVIDER_NAME))
    })
    .await;
    let client = &world.client;
    let anonymous = register(client, json!({ "type": "anonymous" })).await;
    assert_eq!(anonymous.status(), 400);
    assert_eq!(anonymous.json()["error"], "anonymous_not_enabled");
    let service = register(
        client,
        json!({ "type": "service_auth", "login_hint": "a@b.test" }),
    )
    .await;
    assert_eq!(service.json()["error"], "service_auth_not_enabled");

    let off = agent_world(|agent| agent).await;
    let assertion = register(
        &off.client,
        json!({ "type": "identity_assertion", "assertion_type": ID_JAG_TYPE, "assertion": "x.y.z" }),
    )
    .await;
    assert_eq!(assertion.status(), 400);
    assert_eq!(assertion.json()["error"], "issuer_not_enabled");
}

#[tokio::test]
async fn agent_007_unclaimed_agent_holds_only_pre_claim_scopes() {
    let world = agent_world(|agent| agent).await;
    let client = &world.client;
    let registration = anonymous(client).await;
    let assertion = registration["identity_assertion"].as_str().unwrap();
    let wider = exchange(client, assertion, Some("posts:write")).await;
    assert_eq!(wider.status(), 400, "{}", wider.body_text());
    assert_eq!(wider.json()["error"], "invalid_scope");
    let token = agent_token(client, assertion).await;
    let listed = client.mcp().with_token(&token).list().await;
    let names = listed.tool_names();
    assert!(names.contains(&"posts.show".to_owned()), "{names:?}");
    assert!(!names.contains(&"posts.create".to_owned()), "{names:?}");
    let refused = client
        .mcp()
        .with_token(&token)
        .call("posts.create", json!({ "title": "No" }))
        .await;
    assert_eq!(refused.status(), 403);
    let www = refused
        .response()
        .header("www-authenticate")
        .unwrap_or_default()
        .to_owned();
    assert!(www.contains("error=\"insufficient_scope\""), "{www}");
}

#[tokio::test]
async fn agent_007_narrowed_post_claim_scopes_apply_to_a_bound_agent_at_once() {
    let world = agent_world(|agent| agent).await;
    let user = new_account("narrowed");
    let claimed = claim_anonymous(&world, &user).await;
    let token = claimed.tokens["access_token"].as_str().unwrap();
    let assertion = claimed.tokens["identity_assertion"].as_str().unwrap();
    assert_eq!(
        call(
            &world.client,
            token,
            "posts.create",
            json!({ "title": "Before" })
        )
        .await
        .0,
        200
    );

    // The operator narrows the post-claim scopes and restarts.
    let narrowed = world.node(|agent| agent.post_claim_scopes([scope::POSTS_READ]));
    // The token minted before carries posts:write; only the resolver cuts
    // it.
    let refused = narrowed
        .mcp()
        .with_token(token)
        .call("posts.create", json!({ "title": "After" }))
        .await;
    assert_eq!(refused.status(), 403, "{:?}", refused.response());
    let www = refused
        .response()
        .header("www-authenticate")
        .unwrap_or_default()
        .to_owned();
    assert!(www.contains("error=\"insufficient_scope\""), "{www}");
    assert_eq!(
        call(&narrowed, token, "posts.show", json!({ "post": "1" }))
            .await
            .0,
        200,
        "what the configuration still grants keeps working"
    );
    let wider = exchange(&narrowed, assertion, Some("posts:write")).await;
    assert_eq!(wider.status(), 400, "{}", wider.body_text());
    assert_eq!(wider.json()["error"], "invalid_scope");
    let cut = exchange(&narrowed, assertion, None).await;
    assert_eq!(cut.status(), 200, "{}", cut.body_text());
    assert_eq!(cut.json()["scope"], "posts:read");
}

#[tokio::test]
async fn agent_007_post_claim_scopes_default_to_every_declared_scope() {
    let _container = container();
    let db = agent_database().await;
    let resolver = Arc::new(FakeResolver::default());
    let documents = Arc::new(FakeDocuments::default());
    // No scopes named: none before a claim, every declared scope after.
    let router = crate::oauth::router(
        AgentsConfig::new()
            .oauth_server(OAuthServer::new().connection(db))
            .agent_auth(
                AgentAuth::new()
                    .signing_key("service-1", service_pem())
                    .key_set_fetch(resolver, documents),
            ),
    );
    let client = TestClient::new(router, registry());
    let registration = anonymous(&client).await;
    assert_eq!(
        registration["post_claim_scopes"],
        json!(["posts:delete", "posts:read", "posts:write"])
    );
    assert_eq!(registration["pre_claim_scopes"], json!([]));
}

#[tokio::test]
async fn agent_007_assertion_signed_before_a_rotation_still_exchanges() {
    let world = agent_world(|agent| agent).await;
    let old = anonymous(&world.client).await;
    let rotated_pem = service_pem();
    let first_pem = world.pem.clone();
    let rotated = world.node(move |agent| {
        agent
            .signing_key("service-2", &rotated_pem)
            .previous_signing_key("service-1", &first_pem, chrono::Utc::now())
    });
    let tokens = exchange(&rotated, old["identity_assertion"].as_str().unwrap(), None).await;
    assert_eq!(tokens.status(), 200, "{}", tokens.body_text());
    let fresh = anonymous(&rotated).await;
    assert_eq!(
        jwt_part(fresh["identity_assertion"].as_str().unwrap(), 0)["kid"],
        "service-2",
        "new assertions are signed with the new key"
    );
}

#[tokio::test]
async fn agent_007_retired_key_no_longer_verifies() {
    let world = agent_world(|agent| agent).await;
    let old = anonymous(&world.client).await;
    let rotated_pem = service_pem();
    let first_pem = world.pem.clone();
    let rotated = world.node(move |agent| {
        agent
            .signing_key("service-2", &rotated_pem)
            .previous_signing_key(
                "service-1",
                &first_pem,
                chrono::Utc::now() - chrono::Duration::days(31),
            )
    });
    let refused = exchange(&rotated, old["identity_assertion"].as_str().unwrap(), None).await;
    assert_eq!(refused.status(), 400, "{}", refused.body_text());
    assert_eq!(refused.json()["error"], "invalid_grant");
}

#[tokio::test]
async fn agent_007_agent_token_never_calls_a_user_tool() {
    let world = agent_world(|agent| agent).await;
    let user = new_account("user-tool");
    let claimed = claim_anonymous(&world, &user).await;
    let token = claimed.tokens["access_token"].as_str().unwrap();
    let refused = world
        .client
        .mcp()
        .with_token(token)
        .call("users.profile", json!({}))
        .await;
    assert_eq!(refused.status(), 403, "{:?}", refused.response());
    let listed = world.client.mcp().with_token(token).list().await;
    assert!(!listed.tool_names().contains(&"users.profile".to_owned()));
}

#[tokio::test]
async fn agent_007_agent_auth_without_the_oauth_server_fails_the_build() {
    let _container = container();
    let error = Router::new()
        .try_agents(
            AgentsConfig::new().agent_auth(AgentAuth::new().signing_key("k", service_pem())),
        )
        .err()
        .expect("the build fails");
    assert!(
        error
            .to_string()
            .contains("turn on AgentsConfig::oauth_server"),
        "{error}"
    );
}

#[tokio::test]
async fn agent_007_trust_list_without_identity_assertion_fails_the_build() {
    let _container = container();
    let db = agent_database().await;
    let error = Router::new()
        .try_agents(
            AgentsConfig::new()
                .oauth_server(OAuthServer::new().connection(db))
                .agent_auth(
                    AgentAuth::new()
                        .signing_key("k", service_pem())
                        .trust(TrustedIssuer::new(PROVIDER, PROVIDER_JWKS, PROVIDER_NAME)),
                ),
        )
        .err()
        .expect("the build fails");
    assert!(
        error
            .to_string()
            .contains("IdentityType::IdentityAssertion"),
        "{error}"
    );
}

#[tokio::test]
async fn agent_007_missing_signing_key_fails_the_build() {
    let _env = crate::env_lock::lock_env_async().await;
    let _container = container();
    let db = agent_database().await;
    let saved: Vec<(&str, Option<String>)> = ["AGENT_AUTH_KEY_ID", "AGENT_AUTH_PRIVATE_KEY"]
        .into_iter()
        .map(|name| (name, std::env::var(name).ok()))
        .collect();
    // SAFETY: the environment lock is held; no other test reads or writes
    // the environment meanwhile.
    unsafe {
        std::env::remove_var("AGENT_AUTH_KEY_ID");
        std::env::remove_var("AGENT_AUTH_PRIVATE_KEY");
    }
    let missing = Router::new()
        .try_agents(
            AgentsConfig::new()
                .oauth_server(OAuthServer::new().connection(db.clone()))
                .agent_auth(AgentAuth::new()),
        )
        .err()
        .expect("no key fails the build");
    let unreadable = Router::new()
        .try_agents(
            AgentsConfig::new()
                .oauth_server(OAuthServer::new().connection(db))
                .agent_auth(AgentAuth::new().signing_key("k", "not a key")),
        )
        .err()
        .expect("an unreadable key fails the build");
    // SAFETY: as above.
    unsafe {
        for (name, value) in saved {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
    let missing = missing.to_string();
    for part in ["AGENT_AUTH_KEY_ID", "AGENT_AUTH_PRIVATE_KEY", "agents:key"] {
        assert!(missing.contains(part), "{part} in {missing}");
    }
    assert!(unreadable.to_string().contains("P-256"), "{unreadable}");
}

#[tokio::test]
async fn agent_007_signing_keys_come_from_the_environment() {
    let _env = crate::env_lock::lock_env_async().await;
    let _container = container();
    let db = agent_database().await;
    let names = [
        "AGENT_AUTH_KEY_ID",
        "AGENT_AUTH_PRIVATE_KEY",
        "AGENT_AUTH_PREVIOUS_KEY_ID",
        "AGENT_AUTH_PREVIOUS_PRIVATE_KEY",
        "AGENT_AUTH_PREVIOUS_ROTATED_AT",
    ];
    let saved: Vec<(&str, Option<String>)> = names
        .into_iter()
        .map(|name| (name, std::env::var(name).ok()))
        .collect();
    let first = service_pem();
    let second = service_pem();
    // SAFETY: the environment lock is held; no other test reads or writes
    // the environment meanwhile.
    unsafe {
        std::env::set_var("AGENT_AUTH_KEY_ID", "env-1");
        // A literal `\n` stands for a line break, as an environment file
        // writes one.
        std::env::set_var(
            "AGENT_AUTH_PRIVATE_KEY",
            first.trim_end().replace('\n', "\\n"),
        );
        for name in &names[2..] {
            std::env::remove_var(name);
        }
    }
    let first_node = TestClient::new(
        crate::oauth::router(
            AgentsConfig::new()
                .oauth_server(OAuthServer::new().connection(db.clone()))
                .agent_auth(AgentAuth::new()),
        ),
        registry(),
    );
    let old = anonymous(&first_node).await;
    assert_eq!(
        jwt_part(old["identity_assertion"].as_str().unwrap(), 0)["kid"],
        "env-1"
    );
    // SAFETY: as above.
    unsafe {
        std::env::set_var("AGENT_AUTH_KEY_ID", "env-2");
        std::env::set_var("AGENT_AUTH_PRIVATE_KEY", &second);
        std::env::set_var("AGENT_AUTH_PREVIOUS_KEY_ID", "env-1");
        std::env::set_var("AGENT_AUTH_PREVIOUS_PRIVATE_KEY", &first);
        std::env::set_var(
            "AGENT_AUTH_PREVIOUS_ROTATED_AT",
            chrono::Utc::now().to_rfc3339(),
        );
    }
    let rotated = TestClient::new(
        crate::oauth::router(
            AgentsConfig::new()
                .oauth_server(OAuthServer::new().connection(db.clone()))
                .agent_auth(AgentAuth::new()),
        ),
        registry(),
    );
    // SAFETY: as above.
    unsafe {
        std::env::remove_var("AGENT_AUTH_PREVIOUS_ROTATED_AT");
    }
    let partial = Router::new()
        .try_agents(
            AgentsConfig::new()
                .oauth_server(OAuthServer::new().connection(db))
                .agent_auth(AgentAuth::new()),
        )
        .err()
        .expect("a previous key without its rotation time fails the build");
    // SAFETY: as above.
    unsafe {
        for (name, value) in saved {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
    assert!(
        partial
            .to_string()
            .contains("AGENT_AUTH_PREVIOUS_ROTATED_AT"),
        "{partial}"
    );
    let tokens = exchange(&rotated, old["identity_assertion"].as_str().unwrap(), None).await;
    assert_eq!(
        tokens.status(),
        200,
        "the previous key from the environment verifies what it signed: {}",
        tokens.body_text()
    );
}

#[test]
fn agent_007_debug_prints_no_key_material() {
    let pem = service_pem();
    let agent = AgentAuth::new()
        .signing_key("visible-kid", &pem)
        .previous_signing_key("old-kid", &pem, chrono::Utc::now());
    let printed = format!("{agent:?}");
    assert!(
        printed.contains("visible-kid") && printed.contains("old-kid"),
        "{printed}"
    );
    assert!(!printed.contains("PRIVATE KEY"), "{printed}");
    let body: String = pem.lines().nth(1).unwrap().to_owned();
    assert!(!printed.contains(&body), "no line of the key prints");
}

// ---------------------------------------------------------- AGENT-008 ----

/// An account the ID-JAG's email matches, verified.
fn matched_account(label: &str) -> String {
    new_account(label)
}

#[tokio::test]
async fn agent_008_issuer_off_the_list_is_refused() {
    let world = agent_world(with_id_jags).await;
    let rogue = with(
        with(
            id_jag_claims("s", "a@b.test"),
            "iss",
            json!("https://rogue.test"),
        ),
        "client_id",
        json!("https://rogue.test"),
    );
    let refused = present(&world.client, &world.id_jag(&rogue)).await;
    assert_eq!(refused.status(), 400);
    assert_eq!(refused.json()["error"], "invalid_issuer");
}

#[tokio::test]
async fn agent_008_bad_signature_is_refused() {
    let world = agent_world(with_id_jags).await;
    let impostor = IssuerKey::generate("provider-1");
    let forged = impostor.sign_as(
        "provider-1",
        Some("oauth-id-jag+jwt"),
        &id_jag_claims("s", "a@b.test"),
    );
    let refused = present(&world.client, &forged).await;
    assert_eq!(refused.status(), 400, "{}", refused.body_text());
    assert_eq!(refused.json()["error"], "invalid_signature");
}

#[tokio::test]
async fn agent_008_wrong_audience_is_refused() {
    let world = agent_world(with_id_jags).await;
    let claims = with(
        id_jag_claims("s", "a@b.test"),
        "aud",
        json!("https://elsewhere.test"),
    );
    let refused = present(&world.client, &world.id_jag(&claims)).await;
    assert_eq!(refused.json()["error"], "invalid_audience");
}

#[tokio::test]
async fn agent_008_expired_id_jag_is_refused() {
    let world = agent_world(with_id_jags).await;
    let claims = with(id_jag_claims("s", "a@b.test"), "exp", json!(now() - 10));
    let refused = present(&world.client, &world.id_jag(&claims)).await;
    assert_eq!(refused.json()["error"], "expired");
}

#[tokio::test]
async fn agent_008_no_verified_email_or_phone_is_refused() {
    let world = agent_world(with_id_jags).await;
    let unverified = with(
        id_jag_claims("s", "a@b.test"),
        "email_verified",
        json!(false),
    );
    let refused = present(&world.client, &world.id_jag(&unverified)).await;
    assert_eq!(refused.json()["error"], "missing_verified_email");
    let none = without(id_jag_claims("s", "a@b.test"), "email");
    let refused = present(&world.client, &world.id_jag(&none)).await;
    assert_eq!(refused.json()["error"], "missing_verified_email");
}

#[tokio::test]
async fn agent_008_stale_or_missing_auth_time_answers_login_required() {
    let world = agent_world(with_id_jags).await;
    let stale = with(
        id_jag_claims("s", "a@b.test"),
        "auth_time",
        json!(now() - 7200),
    );
    let refused = present(&world.client, &world.id_jag(&stale)).await;
    assert_eq!(refused.status(), 401, "{}", refused.body_text());
    assert_eq!(refused.json()["error"], "login_required");
    assert!(
        refused
            .header("www-authenticate")
            .is_some_and(|www| www.contains("login_required")),
    );
    let missing = without(id_jag_claims("s", "a@b.test"), "auth_time");
    let refused = present(&world.client, &world.id_jag(&missing)).await;
    assert_eq!(refused.status(), 401);
    assert_eq!(refused.json()["error"], "login_required");
}

#[tokio::test]
async fn agent_008_replayed_jti_is_refused_on_a_second_node() {
    let world = agent_world(with_id_jags).await;
    let second = world.node(with_id_jags);
    let user = matched_account("replay");
    let id_jag = world.id_jag(&id_jag_claims("replay-subject", &account_email(&user)));
    let first = present(&world.client, &id_jag).await;
    assert_ne!(
        first.json()["error"],
        "replay_detected",
        "{}",
        first.body_text()
    );
    let replayed = present(&second, &id_jag).await;
    assert_eq!(replayed.status(), 400, "{}", replayed.body_text());
    assert_eq!(replayed.json()["error"], "replay_detected");
}

/// One ID-JAG presented to two nodes at once, each on its own pool of a
/// live database: exactly one is not a replay.
async fn replay_race_on(url_variable: &str) {
    let url = std::env::var(url_variable)
        .unwrap_or_else(|_| panic!("set {url_variable} to a disposable database"));
    let connect = || async {
        let mut options = sea_orm::ConnectOptions::new(url.clone());
        options
            .max_connections(4)
            .min_connections(0)
            .connect_timeout(Duration::from_secs(5))
            .acquire_timeout(Duration::from_secs(5));
        sea_orm::Database::connect(options)
            .await
            .expect("the test database must be reachable")
    };
    let first_pool = connect().await;
    OAuthServer::migrate(&first_pool).await.unwrap();
    AgentAuth::migrate(&first_pool).await.unwrap();
    let world = agent_world_on(first_pool, with_id_jags);
    let second_pool = connect().await;
    let second = TestClient::new(
        agent_router(
            &second_pool,
            &world.pem,
            &world.resolver,
            &world.documents,
            with_id_jags,
        ),
        registry(),
    )
    .with_peer_address(PEER.parse().unwrap());
    for round in 0..5 {
        let id_jag = world.id_jag(&id_jag_claims(
            &unique("race"),
            &format!("{}@nobody.test", unique("race")),
        ));
        let (left, right) =
            tokio::join!(present(&world.client, &id_jag), present(&second, &id_jag));
        let replays = [left.json(), right.json()]
            .iter()
            .filter(|body| body["error"] == "replay_detected")
            .count();
        assert_eq!(replays, 1, "round {round}: exactly one replay");
    }
}

#[tokio::test]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn agent_008_postgres_replayed_jti_is_refused_on_a_second_node() {
    replay_race_on("PG_TEST_URL").await;
}

#[tokio::test]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn agent_008_mysql_replayed_jti_is_refused_on_a_second_node() {
    replay_race_on("MYSQL_TEST_URL").await;
}

#[tokio::test]
async fn agent_008_email_only_match_answers_interaction_required_and_binds_only_after_the_claim() {
    let world = agent_world(with_id_jags).await;
    let client = &world.client;
    let user = matched_account("step-up");
    let response = present(
        client,
        &world.id_jag(&id_jag_claims("step-up-subject", &account_email(&user))),
    )
    .await;
    assert_eq!(response.status(), 401, "{}", response.body_text());
    let body = response.json();
    assert_eq!(body["error"], "interaction_required");
    assert!(
        body.get("identity_assertion").is_none(),
        "nothing binds yet"
    );
    let claim_token = body["claim_token"].as_str().unwrap();
    let pending = poll(client, claim_token).await;
    assert_eq!(pending.json()["error"], "authorization_pending");
    sign_in(client, &user).await;
    let page = open_claim(client, body["claim"]["verification_uri"].as_str().unwrap()).await;
    assert_eq!(page.status(), 200, "{}", page.body_text());
    let done = confirm(client, &page, body["claim"]["user_code"].as_str().unwrap()).await;
    assert_eq!(done.status(), 200, "{}", done.body_text());
    forget_polls(&world.db).await;
    let tokens = poll(client, claim_token).await;
    assert_eq!(tokens.status(), 200, "{}", tokens.body_text());
    let token = tokens.json()["access_token"].as_str().unwrap().to_owned();
    let (status, created) = call(client, &token, "posts.create", json!({ "title": "x" })).await;
    assert_eq!(status, 200, "{created}");
    assert_eq!(created["user"], user);
}

#[tokio::test]
async fn agent_008_known_delegation_binds_without_a_ceremony() {
    let world =
        agent_world(|agent| with_id_jags(agent).provision_users(Arc::new(CreateAccounts))).await;
    let user = unique("delegation");
    let claims = id_jag_claims("known-subject", &account_email(&user));
    let first = present(&world.client, &world.id_jag(&claims)).await;
    assert_eq!(first.status(), 200, "{}", first.body_text());
    // The same issuer and subject again: the delegation is found first.
    let again = present(
        &world.client,
        &world.id_jag(&with(claims, "jti", json!(unique("jti")))),
    )
    .await;
    assert_eq!(again.status(), 200, "{}", again.body_text());
    assert_eq!(
        again.json()["registration_id"],
        first.json()["registration_id"]
    );
}

/// A provisioner that names an account whose email is left unverified.
struct CreateUnverified;

#[async_trait]
impl AgentUserProvisioner for CreateUnverified {
    async fn provision(
        &self,
        identity: &ProvidedIdentity,
    ) -> Result<Option<String>, FrameworkError> {
        let id = identity
            .verified_email
            .as_deref()
            .and_then(|email| email.strip_suffix("@accounts.test"))
            .unwrap_or_default()
            .to_owned();
        remember_account(&id);
        mark_unverified(&id);
        Ok(Some(id))
    }
}

#[tokio::test]
async fn agent_008_no_match_follows_the_provisioning_policy() {
    // The default policy refuses.
    let refusing = agent_world(with_id_jags).await;
    let refused = present(
        &refusing.client,
        &refusing.id_jag(&id_jag_claims("new-1", &account_email(&unique("nobody")))),
    )
    .await;
    assert_eq!(refused.status(), 400, "{}", refused.body_text());
    assert_eq!(refused.json()["error"], "invalid_request");

    // A provisioner that creates the user binds it.
    let creating =
        agent_world(|agent| with_id_jags(agent).provision_users(Arc::new(CreateAccounts))).await;
    let created = present(
        &creating.client,
        &creating.id_jag(&id_jag_claims("new-2", &account_email(&unique("fresh")))),
    )
    .await;
    assert_eq!(created.status(), 200, "{}", created.body_text());

    // A user without the provider's verified email is never bound.
    let unverified =
        agent_world(|agent| with_id_jags(agent).provision_users(Arc::new(CreateUnverified))).await;
    let refused = present(
        &unverified.client,
        &unverified.id_jag(&id_jag_claims(
            "new-3",
            &account_email(&unique("unverified")),
        )),
    )
    .await;
    assert_eq!(refused.status(), 400, "{}", refused.body_text());
}

#[tokio::test]
async fn agent_008_claim_page_shows_the_trust_list_name_never_the_providers() {
    let world = agent_world(with_id_jags).await;
    let client = &world.client;
    let user = matched_account("display-name");
    let claims = with(
        id_jag_claims("named-subject", &account_email(&user)),
        "name",
        json!("Provider-Chosen Name"),
    );
    let response = present(client, &world.id_jag(&claims)).await;
    assert_eq!(response.status(), 401);
    sign_in(client, &user).await;
    let page = open_claim(
        client,
        response.json()["claim"]["verification_uri"]
            .as_str()
            .unwrap(),
    )
    .await;
    let text = page.body_text();
    assert!(text.contains(PROVIDER_NAME), "{text}");
    assert!(!text.contains("Provider-Chosen Name"), "{text}");
    assert!(!text.contains("test-platform"), "{text}");
}

#[tokio::test]
async fn agent_008_key_set_on_a_private_address_is_not_fetched() {
    let world = agent_world(with_id_jags).await;
    world.resolver.answer(PROVIDER_HOST, "10.0.0.7");
    let refused = present(
        &world.client,
        &world.id_jag(&id_jag_claims("private", "a@b.test")),
    )
    .await;
    assert_eq!(refused.status(), 400, "{}", refused.body_text());
    assert_eq!(refused.json()["error"], "invalid_signature");
    assert!(
        world.documents.requests.lock().unwrap().is_empty(),
        "nothing connected to a private address"
    );
}

#[tokio::test]
async fn agent_008_unknown_key_id_refetches_once() {
    let world =
        agent_world(|agent| with_id_jags(agent).provision_users(Arc::new(CreateAccounts))).await;
    let fetches = || {
        world
            .documents
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|url| url.as_str() == PROVIDER_JWKS)
            .count()
    };
    let first = present(
        &world.client,
        &world.id_jag(&id_jag_claims("k-1", &account_email(&unique("k")))),
    )
    .await;
    assert_eq!(first.status(), 200, "{}", first.body_text());
    assert_eq!(fetches(), 1);
    // The provider rotates: a key the cached set lacks refetches once.
    let rotated = IssuerKey::generate("provider-2");
    world.documents.serve(
        PROVIDER_JWKS,
        &json!({ "keys": [world.provider.jwk(), rotated.jwk()] }),
    );
    let signed = rotated.sign_as(
        "provider-2",
        Some("oauth-id-jag+jwt"),
        &id_jag_claims("k-2", &account_email(&unique("k"))),
    );
    let second = present(&world.client, &signed).await;
    assert_eq!(second.status(), 200, "{}", second.body_text());
    assert_eq!(fetches(), 2);
    // Another unknown key right after does not refetch again.
    let unknown = IssuerKey::generate("provider-3").sign_as(
        "provider-3",
        Some("oauth-id-jag+jwt"),
        &id_jag_claims("k-3", "a@b.test"),
    );
    let refused = present(&world.client, &unknown).await;
    assert_eq!(refused.json()["error"], "invalid_signature");
    assert_eq!(fetches(), 2, "one refetch, then the spacing holds");
}

// ---------------------------------------------------------- AGENT-009 ----

#[tokio::test]
async fn agent_009_one_time_values_are_stored_as_digests() {
    let world = agent_world(|agent| agent).await;
    let user = new_account("digests");
    let registration = service_auth(&world.client, &account_email(&user)).await;
    let claim_token = registration["claim_token"].as_str().unwrap();
    let code = registration["claim"]["user_code"].as_str().unwrap();
    let attempt = query_param(
        registration["claim"]["verification_uri"].as_str().unwrap(),
        "claim_attempt_token",
    )
    .unwrap();
    let rows = {
        use sea_orm::EntityTrait;
        magnetar::agent_auth::sql::registrations::Entity::find()
            .all(&world.db)
            .await
            .unwrap()
    };
    let dump = format!("{rows:?}");
    assert!(dump.contains("claim_token_hash"), "{dump}");
    for secret in [claim_token, code, attempt.as_str()] {
        assert!(
            !dump.contains(secret),
            "{secret} is stored only as a digest"
        );
    }
    let digest = |value: &str| {
        use sha2::Digest;
        hex::encode(sha2::Sha256::digest(value.as_bytes()))
    };
    assert!(dump.contains(&digest(claim_token)), "{dump}");
    assert!(dump.contains(&digest(&attempt)), "{dump}");
}

#[tokio::test]
async fn agent_009_claim_token_is_returned_once() {
    let world = agent_world(|agent| agent).await;
    let user = new_account("once");
    let registration = anonymous(&world.client).await;
    let claim_token = registration["claim_token"].as_str().unwrap();
    let started = initiate(&world.client, claim_token, &account_email(&user))
        .await
        .json();
    assert!(started.get("claim_token").is_none(), "{started}");
    assert!(
        !started.to_string().contains(claim_token),
        "the claim token is never sent back"
    );
}

#[tokio::test]
async fn agent_009_user_code_expires_after_its_lifetime() {
    let world = agent_world(|agent| agent.user_code_lifetime(Duration::from_secs(1))).await;
    let client = &world.client;
    let user = new_account("lifetime");
    let registration = service_auth(client, &account_email(&user)).await;
    sign_in(client, &user).await;
    let page = open_claim(
        client,
        registration["claim"]["verification_uri"].as_str().unwrap(),
    )
    .await;
    assert_eq!(page.status(), 200);
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let late = confirm(
        client,
        &page,
        registration["claim"]["user_code"].as_str().unwrap(),
    )
    .await;
    assert_eq!(late.status(), 410, "{}", late.body_text());
}

#[tokio::test]
async fn agent_009_user_code_stops_after_the_wrong_entry_limit() {
    let world = agent_world(|agent| agent.max_user_code_failures(2)).await;
    let client = &world.client;
    let user = new_account("wrong-codes");
    let registration = service_auth(client, &account_email(&user)).await;
    let code = registration["claim"]["user_code"].as_str().unwrap();
    let wrong = if code == "000000" { "111111" } else { "000000" };
    sign_in(client, &user).await;
    let page = open_claim(
        client,
        registration["claim"]["verification_uri"].as_str().unwrap(),
    )
    .await;
    let first = confirm(client, &page, wrong).await;
    assert_eq!(first.status(), 400, "{}", first.body_text());
    assert!(first.body_text().contains("That code is wrong"));
    assert!(
        first.body_text().contains("1 attempts left"),
        "{}",
        first.body_text()
    );
    let second = confirm(client, &page, wrong).await;
    assert_ne!(second.status(), 200);
    let right = confirm(client, &page, code).await;
    assert_eq!(
        right.status(),
        410,
        "the attempt is spent: {}",
        right.body_text()
    );
}

#[tokio::test]
async fn agent_009_user_signed_in_as_another_email_cannot_claim() {
    let world = agent_world(|agent| agent).await;
    let client = &world.client;
    let owner = new_account("owner");
    let other = new_account("other");
    let registration = service_auth(client, &account_email(&owner)).await;
    sign_in(client, &other).await;
    let page = open_claim(
        client,
        registration["claim"]["verification_uri"].as_str().unwrap(),
    )
    .await;
    assert_eq!(page.status(), 403, "{}", page.body_text());
    assert!(
        page.body_text()
            .contains("This claim is for another account")
    );
    // The plugin's own confirm route refuses the same.
    let attempt = query_param(
        registration["claim"]["verification_uri"].as_str().unwrap(),
        "claim_attempt_token",
    )
    .unwrap();
    let token = session_token(client).await;
    let refused = client
        .post("/agent/identity/claim/complete")
        .form(&[
            ("_token", token.as_str()),
            ("claim_attempt_token", attempt.as_str()),
            (
                "user_code",
                registration["claim"]["user_code"].as_str().unwrap(),
            ),
        ])
        .send()
        .await;
    assert_eq!(refused.status(), 403, "{}", refused.body_text());
}

/// The signed-in session's CSRF token.
async fn session_token(client: &TestClient) -> String {
    let response = client.get("/test/csrf").send().await;
    response.body_text()
}

#[tokio::test]
async fn agent_009_unverified_email_cannot_claim() {
    let world = agent_world(|agent| agent).await;
    let client = &world.client;
    let user = new_account("unverified-claimer");
    mark_unverified(&user);
    let registration = service_auth(client, &account_email(&user)).await;
    sign_in(client, &user).await;
    let page = open_claim(
        client,
        registration["claim"]["verification_uri"].as_str().unwrap(),
    )
    .await;
    assert_eq!(page.status(), 403, "{}", page.body_text());
}

#[tokio::test]
async fn agent_009_claim_form_needs_the_csrf_token() {
    // Both claim routes check the token themselves, so an application that
    // installed no CSRF middleware of its own is protected too.
    let world = agent_world(|agent| agent).await;
    let client = &world.node_with(crate::oauth::session_registry(), |agent| agent);
    let user = new_account("csrf");
    let registration = service_auth(client, &account_email(&user)).await;
    sign_in(client, &user).await;
    let page = open_claim(
        client,
        registration["claim"]["verification_uri"].as_str().unwrap(),
    )
    .await;
    assert_eq!(page.status(), 200, "{}", page.body_text());
    let text = page.body_text();
    let attempt = hidden(&text, "claim_attempt_token");
    let code = registration["claim"]["user_code"].as_str().unwrap();
    let framework = client
        .post("/agent/claim")
        .form(&[
            ("claim_attempt_token", attempt.as_str()),
            ("user_code", code),
        ])
        .send()
        .await;
    assert_eq!(framework.status(), 419, "{}", framework.body_text());
    let plugin = client
        .post("/agent/identity/claim/complete")
        .form(&[
            ("claim_attempt_token", attempt.as_str()),
            ("user_code", code),
        ])
        .send()
        .await;
    assert_eq!(plugin.status(), 419, "{}", plugin.body_text());
    // The code still works with the token.
    let done = confirm(client, &page, code).await;
    assert_eq!(done.status(), 200, "{}", done.body_text());
}

#[tokio::test]
async fn agent_009_claim_page_needs_a_signed_in_user() {
    let world = agent_world(|agent| agent).await;
    let client = &world.client;
    let user = new_account("guest-first");
    let registration = service_auth(client, &account_email(&user)).await;
    let uri = registration["claim"]["verification_uri"].as_str().unwrap();
    // A guest opening the verification URI reaches sign-in...
    let guest = open_claim(client, uri).await;
    assert_eq!(guest.status(), 302, "{}", guest.body_text());
    let sign_in_at = location(&guest);
    assert!(
        sign_in_at.starts_with("http://localhost/login?return_to="),
        "{sign_in_at}"
    );
    assert!(
        !guest.body_text().contains("@accounts.test"),
        "nothing shows before sign-in"
    );
    // ... and comes back to the claim page with the attempt.
    let return_to = query_param(&sign_in_at, "return_to").unwrap();
    assert_eq!(return_to, path_of(uri));
    sign_in(client, &user).await;
    let page = client.get(return_to).send().await;
    assert_eq!(page.status(), 200, "{}", page.body_text());
    assert!(page.body_text().contains(&account_email(&user)));
}

#[tokio::test]
async fn agent_009_claim_page_refuses_framing_and_caching() {
    let world = agent_world(|agent| agent).await;
    let client = &world.client;
    let user = new_account("framing");
    let registration = service_auth(client, &account_email(&user)).await;
    let uri = registration["claim"]["verification_uri"].as_str().unwrap();
    let guest = open_claim(client, uri).await;
    sign_in(client, &user).await;
    let page = open_claim(client, uri).await;
    let missing = client.get("/agent/claim").send().await;
    for response in [&guest, &page, &missing] {
        assert_eq!(response.header("x-frame-options"), Some("DENY"));
        assert_eq!(
            response.header("content-security-policy"),
            Some("frame-ancestors 'none'")
        );
        assert_eq!(response.header("cache-control"), Some("no-store"));
        assert_eq!(response.header("referrer-policy"), Some("no-referrer"));
    }
}

#[tokio::test]
async fn agent_009_claim_revokes_pre_claim_tokens_and_returns_an_assertion_with_the_email() {
    let world = agent_world(|agent| agent).await;
    let client = &world.client;
    let user = new_account("pre-claim");
    let registration = anonymous(client).await;
    let pre_claim = agent_token(client, registration["identity_assertion"].as_str().unwrap()).await;
    assert_eq!(
        call(client, &pre_claim, "posts.show", json!({ "post": "1" }))
            .await
            .0,
        200
    );
    let claim_token = registration["claim_token"].as_str().unwrap();
    let attempt = initiate(client, claim_token, &account_email(&user))
        .await
        .json()["claim_attempt"]
        .clone();
    sign_in(client, &user).await;
    let page = open_claim(client, attempt["verification_uri"].as_str().unwrap()).await;
    let done = confirm(client, &page, attempt["user_code"].as_str().unwrap()).await;
    assert_eq!(done.status(), 200, "{}", done.body_text());
    assert_eq!(
        refusal(client, &pre_claim).await,
        "The token was revoked.",
        "the pre-claim token ends with the claim"
    );
    forget_polls(&world.db).await;
    let tokens = poll(client, claim_token).await.json();
    let assertion = tokens["identity_assertion"].as_str().unwrap();
    let claims = jwt_part(assertion, 1);
    assert_eq!(claims["email"], account_email(&user));
    assert_eq!(claims["email_verified"], true);
}

#[tokio::test]
async fn agent_009_claim_dispatches_agent_claimed() {
    let world = agent_world(|agent| agent).await;
    let user = new_account("claimed-event");
    let _events = EventFacade::fake_only(&["AgentClaimed"]);
    let claimed = claim_anonymous(&world, &user).await;
    let registration_id = claimed.registration["registration_id"].as_str().unwrap();
    let events = dispatched::<AgentClaimed>(|_| true);
    assert_eq!(
        events,
        vec![AgentClaimed {
            event_id: outbox_of(&world.db, registration_id).await[0].id.clone(),
            registration_id: registration_id.to_owned(),
            registration_type: "anonymous".to_owned(),
            user_id: user.clone(),
            provider: None,
            client_address: Some(PEER.to_owned()),
        }]
    );
}

/// Every outbox event of a registration.
async fn outbox_of(
    db: &DatabaseConnection,
    registration_id: &str,
) -> Vec<magnetar::agent_auth::sql::agent_events::Model> {
    use magnetar::agent_auth::sql::agent_events;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    agent_events::Entity::find()
        .filter(agent_events::Column::RegistrationId.eq(registration_id.to_owned()))
        .all(db)
        .await
        .expect("read the outbox")
}

#[tokio::test]
async fn agent_009_claim_event_carries_a_stable_id() {
    let world = agent_world(|agent| agent).await;
    let user = new_account("stable-id");
    let _events = EventFacade::fake_only(&["AgentClaimed", "MagnetarLifecycleEvent"]);
    let claimed = claim_anonymous(&world, &user).await;
    let registration_id = claimed.registration["registration_id"].as_str().unwrap();
    let outbox = outbox_of(&world.db, registration_id).await;
    assert_eq!(outbox.len(), 1, "the claim wrote one event");
    assert!(
        outbox[0].delivered_at.is_some(),
        "delivered after the commit"
    );
    let event_id = outbox[0].id.clone();
    assert!(event_id.starts_with("agent-auth:"), "{event_id}");
    let claimed_events = dispatched::<AgentClaimed>(|_| true);
    assert_eq!(claimed_events.len(), 1);
    assert_eq!(claimed_events[0].event_id, event_id);
    let lifecycle = dispatched::<MagnetarLifecycleEvent>(|event| {
        event.kind.audit_name() == Some("claim.confirmed")
    });
    assert_eq!(lifecycle.len(), 1);
    assert_eq!(
        lifecycle[0].mutation_id, event_id,
        "one id on every form of it"
    );
}

/// An `AgentClaimed` listener that fails its first call and records the
/// event id of every call.
#[derive(Default)]
struct FailsFirst {
    calls: Mutex<Vec<String>>,
}

#[async_trait]
impl suprnova::Listener<AgentClaimed> for FailsFirst {
    async fn handle(&self, event: &AgentClaimed) -> Result<(), FrameworkError> {
        let first = {
            let mut calls = self.calls.lock().unwrap();
            calls.push(event.event_id.clone());
            calls.len() == 1
        };
        if first {
            return Err(FrameworkError::internal("the listener's store is down"));
        }
        Ok(())
    }
}

#[test]
fn agent_009_claim_event_survives_a_failed_dispatch() {
    if !crate::own_process::is_child() {
        crate::own_process::run_alone(
            "agent_auth::agent_009_claim_event_survives_a_failed_dispatch",
        );
        return;
    }
    // SAFETY: this test runs alone in its own process.
    unsafe {
        std::env::set_var("APP_ENV", "testing");
        std::env::set_var("APP_URL", ISSUER);
    }
    suprnova::boot::load_env().expect("pre-runtime configuration");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(async {
            let listener = Arc::new(FailsFirst::default());
            EventFacade::listen::<AgentClaimed, _>(listener.clone()).await;
            let world = agent_world(|agent| agent).await;
            let user = new_account("failed-dispatch");
            // The claim stands though its event's first dispatch fails.
            let claimed = claim_anonymous(&world, &user).await;
            let registration_id = claimed.registration["registration_id"]
                .as_str()
                .unwrap()
                .to_owned();
            let outbox = outbox_of(&world.db, &registration_id).await;
            assert_eq!(outbox.len(), 1);
            assert!(outbox[0].delivered_at.is_none(), "the event is not lost");
            let event_id = outbox[0].id.clone();
            assert_eq!(
                listener.calls.lock().unwrap().clone(),
                vec![event_id.clone()]
            );

            let (db, pem, resolver, documents) = (
                world.db.clone(),
                world.pem.clone(),
                world.resolver.clone(),
                world.documents.clone(),
            );
            suprnova::Application::new()
                .routes(move || agent_router(&db, &pem, &resolver, &documents, |agent| agent))
                .run_with_args(["app", "agents:prune"])
                .await
                .expect("agents:prune runs");
            assert_eq!(
                listener.calls.lock().unwrap().clone(),
                vec![event_id.clone(), event_id.clone()],
                "the pass fires AgentClaimed again, with the same event id"
            );
            assert!(
                outbox_of(&world.db, &registration_id).await[0]
                    .delivered_at
                    .is_some()
            );
        });
}

/// Run `body` as the only test of a child process, with the environment a
/// booted command such as `agents:prune` needs. `test` is the test's name.
fn alone(test: &str, body: impl std::future::Future<Output = ()>) {
    if !crate::own_process::is_child() {
        crate::own_process::run_alone(&format!("agent_auth::{test}"));
        return;
    }
    // SAFETY: this test runs alone in its own process.
    unsafe {
        std::env::set_var("APP_ENV", "testing");
        std::env::set_var("APP_URL", ISSUER);
    }
    suprnova::boot::load_env().expect("pre-runtime configuration");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(body);
}

/// Run `agents:prune` with `extra` arguments over a router of `world`'s
/// database, its agent auth as `configure` adjusts it.
async fn agents_prune(
    world: &AgentWorld,
    configure: impl FnOnce(AgentAuth) -> AgentAuth + Send + 'static,
    extra: &[&'static str],
) {
    let (db, pem, resolver, documents) = (
        world.db.clone(),
        world.pem.clone(),
        world.resolver.clone(),
        world.documents.clone(),
    );
    let mut args = vec!["app", "agents:prune"];
    args.extend_from_slice(extra);
    suprnova::Application::new()
        .routes(move || agent_router(&db, &pem, &resolver, &documents, configure))
        .run_with_args(args)
        .await
        .expect("agents:prune runs");
}

/// An `AgentClaimed` listener that fails until it is told to work, with an
/// error naming a detail the outbox must not keep.
#[derive(Default)]
struct FailsUntilFixed {
    working: std::sync::atomic::AtomicBool,
    calls: Mutex<Vec<String>>,
}

#[async_trait]
impl suprnova::Listener<AgentClaimed> for FailsUntilFixed {
    async fn handle(&self, event: &AgentClaimed) -> Result<(), FrameworkError> {
        self.calls.lock().unwrap().push(event.event_id.clone());
        if self.working.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(());
        }
        Err(FrameworkError::internal(format!(
            "secret-detail: cannot move rows for {}",
            event.user_id
        )))
    }
}

/// Claim a registration as a new user while `listener` fails, and run
/// `agents:prune` until the claim event used every attempt. Answers the
/// registration id and the user.
async fn claim_event_failed_for_good(
    world: &AgentWorld,
    listener: &FailsUntilFixed,
) -> (String, String) {
    let user = new_account("failed-for-good");
    let claimed = claim_anonymous(world, &user).await;
    let registration_id = claimed.registration["registration_id"]
        .as_str()
        .unwrap()
        .to_owned();
    // The delivery after the commit was the first attempt.
    for _ in 1..12 {
        agents_prune(world, |agent| agent, &[]).await;
    }
    assert_eq!(listener.calls.lock().unwrap().len(), 12, "twelve attempts");
    (registration_id, user)
}

#[test]
fn agent_009_claim_event_that_always_fails_is_marked_failed_and_not_delivered_again() {
    alone(
        "agent_009_claim_event_that_always_fails_is_marked_failed_and_not_delivered_again",
        async {
            let listener = Arc::new(FailsUntilFixed::default());
            EventFacade::listen::<AgentClaimed, _>(listener.clone()).await;
            let world = agent_world(|agent| agent).await;
            let (registration_id, user) = claim_event_failed_for_good(&world, &listener).await;
            let row = outbox_of(&world.db, &registration_id).await.remove(0);
            assert_eq!(row.attempts, 12);
            assert!(row.failed_at.is_some(), "failed after the limit");
            assert!(row.delivered_at.is_none());
            let last_error = row.last_error.unwrap_or_default();
            assert!(last_error.contains("AgentClaimed"), "{last_error}");
            assert!(
                !last_error.contains("secret-detail") && !last_error.contains(&user),
                "the outbox keeps no listener detail: {last_error}"
            );
            agents_prune(&world, |agent| agent, &[]).await;
            assert_eq!(
                listener.calls.lock().unwrap().len(),
                12,
                "a failed event is not delivered again"
            );
        },
    );
}

#[test]
fn agent_009_retry_failed_delivers_a_failed_claim_event_once_the_listener_works() {
    alone(
        "agent_009_retry_failed_delivers_a_failed_claim_event_once_the_listener_works",
        async {
            let listener = Arc::new(FailsUntilFixed::default());
            EventFacade::listen::<AgentClaimed, _>(listener.clone()).await;
            let world = agent_world(|agent| agent).await;
            let (registration_id, _) = claim_event_failed_for_good(&world, &listener).await;
            let event_id = outbox_of(&world.db, &registration_id).await[0].id.clone();
            listener
                .working
                .store(true, std::sync::atomic::Ordering::SeqCst);
            agents_prune(&world, |agent| agent, &["--retry-failed"]).await;
            let calls = listener.calls.lock().unwrap().clone();
            assert_eq!(calls.len(), 13, "one more delivery");
            assert_eq!(calls[12], event_id, "the same event");
            let row = outbox_of(&world.db, &registration_id).await.remove(0);
            assert!(row.delivered_at.is_some());
            assert!(row.failed_at.is_none());
        },
    );
}

/// Counts the claim's `MagnetarLifecycleEvent`s.
#[derive(Default)]
struct CountsClaimLifecycle(std::sync::atomic::AtomicUsize);

#[async_trait]
impl suprnova::Listener<MagnetarLifecycleEvent> for CountsClaimLifecycle {
    async fn handle(&self, event: &MagnetarLifecycleEvent) -> Result<(), FrameworkError> {
        if event.kind.audit_name() == Some("claim.confirmed") {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(())
    }
}

#[test]
fn agent_009_claim_event_retry_writes_no_second_audit_record() {
    alone(
        "agent_009_claim_event_retry_writes_no_second_audit_record",
        async {
            let sink = Arc::new(MemorySink::default());
            let driver = unique("retry-audit-driver");
            let channel = unique("retry-audit");
            let shared = sink.clone();
            suprnova::Log::extend(&driver, move |_| {
                Ok(shared.clone() as Arc<dyn suprnova::LogSink>)
            });
            suprnova::Log::define(&channel, suprnova::LogChannel::driver(&driver));
            let listener = Arc::new(FailsFirst::default());
            EventFacade::listen::<AgentClaimed, _>(listener.clone()).await;
            let lifecycle = Arc::new(CountsClaimLifecycle::default());
            EventFacade::listen::<MagnetarLifecycleEvent, _>(lifecycle.clone()).await;
            let named = channel.clone();
            let world = agent_world(move |agent| agent.audit_log_channel(named)).await;
            let user = new_account("retry-audit");
            let claimed = claim_anonymous(&world, &user).await;
            let registration_id = claimed.registration["registration_id"]
                .as_str()
                .unwrap()
                .to_owned();
            let event_id = outbox_of(&world.db, &registration_id).await[0].id.clone();
            let again = channel.clone();
            agents_prune(&world, move |agent| agent.audit_log_channel(again), &[]).await;
            assert_eq!(
                listener.calls.lock().unwrap().len(),
                2,
                "the retry delivered it"
            );

            let records = sink.records.lock().unwrap().clone();
            let claim_records = records
                .iter()
                .filter(|record| record.message == "agent auth: claim.confirmed")
                .count();
            assert_eq!(claim_records, 1, "one audit record: {records:?}");
            let warnings: Vec<_> = records
                .iter()
                .filter(|record| record.level == suprnova::LogLevel::Warning)
                .collect();
            assert_eq!(warnings.len(), 1, "one warning for the retry: {records:?}");
            let field = |name: &str| {
                warnings[0]
                    .context
                    .iter()
                    .find(|(key, _)| key == name)
                    .map(|(_, value)| value.clone())
            };
            assert_eq!(field("event_id").as_deref(), Some(event_id.as_str()));
            assert_eq!(field("attempt").as_deref(), Some("2"));
            assert_eq!(
                lifecycle.0.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "one MagnetarLifecycleEvent for the claim"
            );
        },
    );
}

#[tokio::test]
async fn agent_009_reinitiation_with_the_same_claim_token_mints_a_new_code() {
    let world = agent_world(|agent| agent).await;
    let user = new_account("again");
    let registration = anonymous(&world.client).await;
    let claim_token = registration["claim_token"].as_str().unwrap();
    let first = initiate(&world.client, claim_token, &account_email(&user))
        .await
        .json();
    let second = initiate(&world.client, claim_token, &account_email(&user))
        .await
        .json();
    assert_ne!(
        first["claim_attempt"]["verification_uri"],
        second["claim_attempt"]["verification_uri"]
    );
    sign_in(&world.client, &user).await;
    let old = open_claim(
        &world.client,
        first["claim_attempt"]["verification_uri"].as_str().unwrap(),
    )
    .await;
    assert_eq!(
        old.status(),
        404,
        "the first attempt ended: {}",
        old.body_text()
    );
    let page = open_claim(
        &world.client,
        second["claim_attempt"]["verification_uri"]
            .as_str()
            .unwrap(),
    )
    .await;
    let done = confirm(
        &world.client,
        &page,
        second["claim_attempt"]["user_code"].as_str().unwrap(),
    )
    .await;
    assert_eq!(done.status(), 200, "{}", done.body_text());
}

#[tokio::test]
async fn agent_009_polling_answers_pending_slow_down_and_expired_token() {
    let world = agent_world(|agent| agent.registration_window(Duration::from_secs(2))).await;
    let user = new_account("polling");
    let registration = service_auth(&world.client, &account_email(&user)).await;
    let claim_token = registration["claim_token"].as_str().unwrap();
    let pending = poll(&world.client, claim_token).await;
    assert_eq!(pending.status(), 400);
    assert_eq!(pending.json()["error"], "authorization_pending");
    let fast = poll(&world.client, claim_token).await;
    assert_eq!(fast.json()["error"], "slow_down");
    tokio::time::sleep(Duration::from_millis(2200)).await;
    forget_polls(&world.db).await;
    let expired = poll(&world.client, claim_token).await;
    assert_eq!(
        expired.json()["error"],
        "expired_token",
        "{}",
        expired.body_text()
    );
}

#[tokio::test]
async fn agent_009_custom_claim_page_uses_the_plugin_routes() {
    let world = agent_world(|agent| agent.custom_claim_page("/my/claim")).await;
    let client = &world.client;
    let user = new_account("custom-page");
    let registration = service_auth(client, &account_email(&user)).await;
    let uri = registration["claim"]["verification_uri"].as_str().unwrap();
    assert!(
        uri.starts_with("http://localhost/my/claim?claim_attempt_token="),
        "{uri}"
    );
    assert_eq!(
        client.get("/agent/claim").send().await.status(),
        404,
        "no framework page"
    );
    let attempt = query_param(uri, "claim_attempt_token").unwrap();
    sign_in(client, &user).await;
    let pending = client
        .get(format!(
            "/agent/identity/claim/attempt?claim_attempt_token={attempt}"
        ))
        .send()
        .await;
    assert_eq!(pending.status(), 200, "{}", pending.body_text());
    let pending = pending.json();
    assert_eq!(pending["signed_in_as"], account_email(&user));
    assert_eq!(
        pending["scopes"][1],
        json!({ "name": "posts:write", "description": "Create and edit your posts" })
    );
    let token = session_token(client).await;
    let done = client
        .post("/agent/identity/claim/complete")
        .form(&[
            ("_token", token.as_str()),
            ("claim_attempt_token", attempt.as_str()),
            (
                "user_code",
                registration["claim"]["user_code"].as_str().unwrap(),
            ),
        ])
        .send()
        .await;
    assert_eq!(done.status(), 200, "{}", done.body_text());
    assert_eq!(done.json()["status"], "claimed");
}

// ---------------------------------------------------------- AGENT-010 ----

#[tokio::test]
async fn agent_010_revoked_agent_token_stops_calling_tools() {
    let world = agent_world(|agent| agent).await;
    let client = &world.client;
    let registration = anonymous(client).await;
    let token = agent_token(client, registration["identity_assertion"].as_str().unwrap()).await;
    assert_eq!(
        call(client, &token, "posts.show", json!({ "post": "1" }))
            .await
            .0,
        200
    );
    let revoked = client
        .post("/oauth/revoke")
        .form(&[
            ("token", token.as_str()),
            ("token_type_hint", "access_token"),
        ])
        .send()
        .await;
    assert_eq!(revoked.status(), 200, "{}", revoked.body_text());
    assert_eq!(refusal(client, &token).await, "The token was revoked.");
}

#[tokio::test]
async fn agent_010_revoked_registration_with_a_live_grant_is_refused() {
    let world = agent_world(|agent| agent).await;
    let client = &world.client;
    let registration = anonymous(client).await;
    let token = agent_token(client, registration["identity_assertion"].as_str().unwrap()).await;
    assert_eq!(
        call(client, &token, "posts.show", json!({ "post": "1" }))
            .await
            .0,
        200
    );
    // The registration is revoked and its grant left live, as a grant
    // revocation that failed partway leaves it.
    let id = registration["registration_id"].as_str().unwrap();
    world
        .db
        .execute_unprepared(&format!(
            "UPDATE auth_agent_registrations SET status = 'revoked' WHERE id = '{id}'"
        ))
        .await
        .unwrap();
    assert_eq!(
        refusal(client, &token).await,
        "The agent registration was revoked or has ended."
    );
}

#[tokio::test]
async fn agent_010_revoking_an_unknown_token_answers_200() {
    let world = agent_world(|agent| agent).await;
    let revoked = world
        .client
        .post("/oauth/revoke")
        .form(&[("token", "mat_unknown")])
        .send()
        .await;
    assert_eq!(revoked.status(), 200, "{}", revoked.body_text());
}

#[tokio::test]
async fn agent_010_security_event_revokes_the_registration_and_its_tokens() {
    let world =
        agent_world(|agent| with_id_jags(agent).provision_users(Arc::new(CreateAccounts))).await;
    let client = &world.client;
    let subject = unique("set-subject");
    let registration = present(
        client,
        &world.id_jag(&id_jag_claims(&subject, &account_email(&unique("set")))),
    )
    .await
    .json();
    let assertion = registration["identity_assertion"]
        .as_str()
        .unwrap()
        .to_owned();
    let token = agent_token(client, &assertion).await;
    let pushed = client
        .post("/agent/event/notify")
        .body(
            "application/secevent+jwt",
            world.event(&revocation_event(&subject)),
        )
        .send()
        .await;
    assert_eq!(pushed.status(), 202, "{}", pushed.body_text());
    assert_eq!(refusal(client, &token).await, "The token was revoked.");
    let exchanged = exchange(client, &assertion, None).await;
    assert_eq!(exchanged.json()["error"], "invalid_grant");
}

#[tokio::test]
async fn agent_010_forged_and_replayed_security_events_are_refused() {
    let world =
        agent_world(|agent| with_id_jags(agent).provision_users(Arc::new(CreateAccounts))).await;
    let client = &world.client;
    let subject = unique("forged-subject");
    let registration = present(
        client,
        &world.id_jag(&id_jag_claims(&subject, &account_email(&unique("forged")))),
    )
    .await
    .json();
    let token = agent_token(client, registration["identity_assertion"].as_str().unwrap()).await;
    let impostor = IssuerKey::generate("provider-1");
    let forged = impostor.sign_as(
        "provider-1",
        Some("secevent+jwt"),
        &revocation_event(&subject),
    );
    let refused = client
        .post("/agent/event/notify")
        .body("application/secevent+jwt", forged)
        .send()
        .await;
    assert_eq!(refused.status(), 400, "{}", refused.body_text());
    assert_eq!(
        call(client, &token, "posts.show", json!({ "post": "1" }))
            .await
            .0,
        200
    );
    // A genuine event for another subject, replayed: the second is refused.
    let other = world.event(&revocation_event("someone-else"));
    let first = client
        .post("/agent/event/notify")
        .body("application/secevent+jwt", other.clone())
        .send()
        .await;
    assert_eq!(first.status(), 202, "{}", first.body_text());
    let replayed = client
        .post("/agent/event/notify")
        .body("application/secevent+jwt", other)
        .send()
        .await;
    assert_eq!(replayed.status(), 400, "{}", replayed.body_text());
}

#[tokio::test]
async fn agent_010_user_lists_and_revokes_an_agent_registration() {
    let world = agent_world(|agent| agent).await;
    let client = &world.client;
    let user = new_account("page-revoke");
    let claimed = claim_anonymous(&world, &user).await;
    let token = claimed.tokens["access_token"].as_str().unwrap();
    let page = client.get("/oauth/connected").send().await;
    let text = page.body_text();
    assert!(text.contains("<h2>Agents</h2>"), "{text}");
    let id = claimed.registration["registration_id"].as_str().unwrap();
    assert!(
        text.contains(&format!("/oauth/connected/agents/{id}/revoke")),
        "{text}"
    );
    assert!(text.contains("Create and edit your posts"), "{text}");
    let revoked = client
        .post(format!("/oauth/connected/agents/{id}/revoke"))
        .form(&[("_token", form_token(&text).as_str())])
        .send()
        .await;
    assert_eq!(revoked.status(), 303, "{}", revoked.body_text());
    assert_eq!(location(&revoked), "http://localhost/oauth/connected");
    assert_eq!(refusal(client, token).await, "The token was revoked.");
    let after = client.get("/oauth/connected").send().await.body_text();
    assert!(after.contains("No agents can act for you."), "{after}");
    assert!(after.contains("No apps can act for you."), "{after}");
}

#[tokio::test]
async fn agent_010_connected_agents_facade_lists_and_revokes_agent_accounts() {
    let world = agent_world(|agent| agent).await;
    let client = &world.client;
    let user = new_account("facade");
    let claimed = claim_anonymous(&world, &user).await;
    let id = claimed.registration["registration_id"].as_str().unwrap();
    let listed = client.get("/test/facade/agents").send().await;
    assert_eq!(listed.status(), 200, "{}", listed.body_text());
    let accounts = listed.json();
    assert_eq!(accounts[0]["id"], id);
    assert_eq!(accounts[0]["registration_type"], "anonymous");
    assert_eq!(accounts[0]["scopes"][0]["name"], "posts:read");
    assert!(accounts[0]["claimed_at"].is_string());
    let revoked = client
        .get(format!("/test/facade/agents/revoke?registration={id}"))
        .send()
        .await
        .json();
    assert_eq!(revoked["revoked"], true);
    let again = client
        .get(format!("/test/facade/agents/revoke?registration={id}"))
        .send()
        .await
        .json();
    assert_eq!(again["revoked"], false);
    assert_eq!(
        client.get("/test/facade/agents").send().await.json(),
        json!([])
    );
}

#[tokio::test]
async fn agent_010_another_users_registration_cannot_be_revoked() {
    let world = agent_world(|agent| agent).await;
    let client = &world.client;
    let owner = new_account("owner-agent");
    let claimed = claim_anonymous(&world, &owner).await;
    let token = claimed.tokens["access_token"].as_str().unwrap().to_owned();
    let id = claimed.registration["registration_id"].as_str().unwrap();
    let intruder = new_account("intruder");
    sign_in(client, &intruder).await;
    let revoked = client
        .get(format!("/test/facade/agents/revoke?registration={id}"))
        .send()
        .await
        .json();
    assert_eq!(revoked["revoked"], false);
    let refused = client
        .post(format!("/oauth/connected/agents/{id}/revoke"))
        .form(&[("_token", session_token(client).await.as_str())])
        .send()
        .await;
    assert_eq!(refused.status(), 404, "{}", refused.body_text());
    assert_eq!(
        call(client, &token, "posts.show", json!({ "post": "1" }))
            .await
            .0,
        200
    );
}

#[tokio::test]
async fn agent_010_password_change_ends_a_bound_agent_token() {
    let world = agent_world(|agent| agent).await;
    let user = new_account("epoch");
    let claimed = claim_anonymous(&world, &user).await;
    let token = claimed.tokens["access_token"].as_str().unwrap();
    assert_eq!(
        call(&world.client, token, "posts.show", json!({ "post": "1" }))
            .await
            .0,
        200
    );
    crate::oauth::change_password(&user);
    assert!(
        refusal(&world.client, token)
            .await
            .contains("ended with the user's sign-in"),
    );
}

#[tokio::test]
async fn agent_010_agent_revoke_form_needs_the_csrf_token() {
    // The revoke route checks the token itself, so an application that
    // installed no CSRF middleware of its own is protected too.
    let world = agent_world(|agent| agent).await;
    let client = world.node_with(crate::oauth::session_registry(), |agent| agent);
    let user = new_account("revoke-csrf");
    let claimed = claim_anonymous_on(&client, &world.db, &user).await;
    let token = claimed.tokens["access_token"].as_str().unwrap();
    let id = claimed.registration["registration_id"].as_str().unwrap();
    let refused = client
        .post(format!("/oauth/connected/agents/{id}/revoke"))
        .form(&[("padding", "x")])
        .send()
        .await;
    assert_eq!(refused.status(), 419, "{}", refused.body_text());
    assert_eq!(
        call(&client, token, "posts.show", json!({ "post": "1" }))
            .await
            .0,
        200,
        "nothing was revoked"
    );
}

/// The four headers every answer of the default pages carries.
fn assert_guarded(response: &TestResponse) {
    assert_eq!(response.header("cache-control"), Some("no-store"));
    assert_eq!(response.header("x-frame-options"), Some("DENY"));
    assert_eq!(
        response.header("content-security-policy"),
        Some("frame-ancestors 'none'")
    );
    assert_eq!(response.header("referrer-policy"), Some("no-referrer"));
}

#[tokio::test]
async fn agent_009_claim_page_store_failure_keeps_the_protective_headers() {
    let world = agent_world(|agent| agent).await;
    let client = &world.client;
    let user = new_account("claim-failure");
    let registration = service_auth(client, &account_email(&user)).await;
    sign_in(client, &user).await;
    world
        .db
        .execute_unprepared("DROP TABLE auth_agent_registrations")
        .await
        .unwrap();
    let page = open_claim(
        client,
        registration["claim"]["verification_uri"].as_str().unwrap(),
    )
    .await;
    assert_eq!(page.status(), 503, "{}", page.body_text());
    assert_guarded(&page);
}

#[tokio::test]
async fn agent_010_connected_page_store_failure_keeps_the_protective_headers() {
    let world = agent_world(|agent| agent).await;
    let client = &world.client;
    sign_in(client, &new_account("connected-failure")).await;
    world
        .db
        .execute_unprepared("DROP TABLE auth_agent_registrations")
        .await
        .unwrap();
    let page = client.get("/oauth/connected").send().await;
    assert_eq!(page.status(), 503, "{}", page.body_text());
    assert_guarded(&page);
}

/// A provisioner that answers with an account that already exists, whose
/// email is not the provider's.
struct ReturnsExisting(String);

#[async_trait]
impl AgentUserProvisioner for ReturnsExisting {
    async fn provision(
        &self,
        _identity: &ProvidedIdentity,
    ) -> Result<Option<String>, FrameworkError> {
        Ok(Some(self.0.clone()))
    }
}

#[tokio::test]
async fn agent_008_provisioned_user_with_another_email_is_refused() {
    let other = new_account("someone-else");
    let world = agent_world(|agent| {
        with_id_jags(agent).provision_users(Arc::new(ReturnsExisting(other.clone())))
    })
    .await;
    let refused = present(
        &world.client,
        &world.id_jag(&id_jag_claims(
            "mismatch",
            &account_email(&unique("asserted")),
        )),
    )
    .await;
    assert_eq!(refused.status(), 400, "{}", refused.body_text());
    assert!(refused.json().get("identity_assertion").is_none());
}

// ---------------------------------------------------------- AGENT-011 ----

#[tokio::test]
async fn agent_011_auth_md_is_markdown_from_the_configuration() {
    let world = agent_world(with_id_jags).await;
    let document = world.client.get("/auth.md").send().await;
    assert_eq!(document.status(), 200);
    assert_eq!(
        document.header("content-type"),
        Some("text/markdown; charset=utf-8")
    );
    assert!(
        document.header("set-cookie").is_none(),
        "no session for a document"
    );
    let text = document.body_text();
    for part in [
        "http://localhost/agent/identity",
        "http://localhost/oauth/token",
        "http://localhost/oauth/revoke",
        "http://localhost/agent/event/notify",
        "### anonymous",
        "### service_auth",
        "### identity_assertion",
        PROVIDER_NAME,
        "`posts:write`: Create and edit your posts",
        CLAIM_GRANT,
    ] {
        assert!(text.contains(part), "{part} in {text}");
    }
    assert!(
        !text.contains("posts:delete"),
        "an undeclared post-claim scope"
    );
}

#[tokio::test]
async fn agent_011_auth_md_names_only_enabled_types_and_served_endpoints() {
    let world = agent_world(|agent| agent.disable(IdentityType::ServiceAuth)).await;
    let text = world.client.get("/auth.md").send().await.body_text();
    assert!(text.contains("### anonymous"), "{text}");
    assert!(!text.contains("### service_auth"), "{text}");
    assert!(!text.contains("### identity_assertion"), "{text}");
    assert!(!text.contains("/agent/event/notify"), "{text}");
    // No client route answers: the post meets the CSRF check every route a
    // plugin does not name as a client route gets, where a served events
    // route would answer the bad token itself.
    let events = world
        .client
        .post("/agent/event/notify")
        .body("application/secevent+jwt", "x.y.z")
        .send()
        .await;
    assert_eq!(events.status(), 419, "the events route is not served");
    assert_eq!(
        world
            .client
            .get("/agent/event/notify")
            .send()
            .await
            .status(),
        404
    );
}

#[tokio::test]
async fn agent_011_anonymous_registration_is_limited_per_address() {
    let world = agent_world(|agent| agent).await;
    for _ in 0..5 {
        anonymous(&world.client).await;
    }
    let refused = register(&world.client, json!({ "type": "anonymous" })).await;
    assert_eq!(refused.status(), 429, "{}", refused.body_text());
    assert_eq!(refused.json()["error"], "rate_limited");
    assert!(refused.header("retry-after").is_some());
}

#[tokio::test]
async fn agent_011_anonymous_registration_is_limited_per_application() {
    let world = agent_world(|agent| {
        agent.registration_limit(
            IdentityType::Anonymous,
            CallLimit::per_hour(5),
            CallLimit::per_hour(3),
        )
    })
    .await;
    for peer in ["198.51.100.1", "198.51.100.2", "198.51.100.3"] {
        let client = world
            .client
            .clone()
            .with_peer_address(peer.parse().unwrap());
        anonymous(&client).await;
    }
    let fourth = world
        .client
        .clone()
        .with_peer_address("198.51.100.4".parse().unwrap());
    let refused = register(&fourth, json!({ "type": "anonymous" })).await;
    assert_eq!(refused.status(), 429, "{}", refused.body_text());
}

#[tokio::test]
async fn agent_011_id_jag_registration_is_limited() {
    let world = agent_world(|agent| {
        with_id_jags(agent).registration_limit(
            IdentityType::IdentityAssertion,
            CallLimit::per_hour(2),
            CallLimit::per_hour(1000),
        )
    })
    .await;
    for _ in 0..2 {
        let answered = present(&world.client, "not.a.jag").await;
        assert_ne!(answered.status(), 429);
    }
    let refused = present(&world.client, "not.a.jag").await;
    assert_eq!(refused.status(), 429, "{}", refused.body_text());
}

#[tokio::test]
async fn agent_011_client_sent_client_ip_does_not_move_the_budget() {
    let world = agent_world(|agent| agent).await;
    for index in 0..5 {
        let answered = world
            .client
            .post("/agent/identity")
            .header("X-Client-IP", format!("192.0.2.{index}"))
            .json(&json!({ "type": "anonymous" }))
            .send()
            .await;
        assert_eq!(answered.status(), 200);
    }
    let refused = world
        .client
        .post("/agent/identity")
        .header("X-Client-IP", "192.0.2.99")
        .json(&json!({ "type": "anonymous" }))
        .send()
        .await;
    assert_eq!(refused.status(), 429, "the budget is the peer's");
}

#[tokio::test]
async fn agent_011_client_endpoints_need_no_csrf_token() {
    // A signed-in browser carries a session; the client endpoints still
    // need no token, and set no cookie.
    let world = agent_world(with_id_jags).await;
    let client = &world.client;
    sign_in(client, &new_account("cookie-holder")).await;
    let identity = register(client, json!({ "type": "anonymous" })).await;
    assert_eq!(identity.status(), 200, "{}", identity.body_text());
    assert!(identity.header("set-cookie").is_none());
    let claim_token = identity.json()["claim_token"].as_str().unwrap().to_owned();
    let claim = initiate(client, &claim_token, "someone@accounts.test").await;
    assert_ne!(claim.status(), 419, "{}", claim.body_text());
    let events = client
        .post("/agent/event/notify")
        .body("application/secevent+jwt", "x.y.z")
        .send()
        .await;
    assert_ne!(events.status(), 419, "{}", events.body_text());
    let token = exchange(client, "x.y.z", None).await;
    assert_ne!(token.status(), 419, "{}", token.body_text());
    let document = client.get("/auth.md").send().await;
    assert!(document.header("set-cookie").is_none());
}

#[tokio::test]
async fn agent_011_every_agent_endpoint_bounds_its_body() {
    let world = agent_world(with_id_jags).await;
    let client = &world.client;
    let big = "x".repeat(20 * 1024);
    for path in ["/agent/identity", "/agent/identity/claim"] {
        let refused = client
            .post(path)
            .json(&json!({ "type": "anonymous", "padding": big }))
            .send()
            .await;
        assert_eq!(refused.status(), 413, "{path}: {}", refused.body_text());
    }
    let events = client
        .post("/agent/event/notify")
        .body("application/secevent+jwt", big.clone())
        .send()
        .await;
    assert_eq!(events.status(), 413, "{}", events.body_text());
    let user = new_account("bounded");
    let registration = service_auth(client, &account_email(&user)).await;
    sign_in(client, &user).await;
    let page = open_claim(
        client,
        registration["claim"]["verification_uri"].as_str().unwrap(),
    )
    .await;
    let text = page.body_text();
    for path in ["/agent/claim", "/agent/identity/claim/complete"] {
        let refused = client
            .post(path)
            .form(&[
                ("_token", form_token(&text).as_str()),
                (
                    "claim_attempt_token",
                    hidden(&text, "claim_attempt_token").as_str(),
                ),
                ("padding", big.as_str()),
            ])
            .send()
            .await;
        assert_eq!(refused.status(), 413, "{path}: {}", refused.body_text());
    }
}

/// A sink that keeps what it is given.
#[derive(Default)]
struct MemorySink {
    records: Mutex<Vec<suprnova::LogRecord>>,
}

impl suprnova::LogSink for MemorySink {
    fn write(&self, record: &suprnova::LogRecord) -> std::io::Result<()> {
        self.records.lock().unwrap().push(record.clone());
        Ok(())
    }
}

#[tokio::test]
async fn agent_011_audit_records_go_to_the_named_channel_as_fields() {
    let sink = Arc::new(MemorySink::default());
    let driver = unique("agent-audit-driver");
    let channel = unique("agent-audit");
    let shared = sink.clone();
    suprnova::Log::extend(&driver, move |_| {
        Ok(shared.clone() as Arc<dyn suprnova::LogSink>)
    });
    suprnova::Log::define(&channel, suprnova::LogChannel::driver(&driver));
    let named = channel.clone();
    let world = agent_world(move |agent| agent.audit_log_channel(named)).await;
    let registration = register(&world.client, json!({ "type": "anonymous" })).await;
    assert_eq!(registration.status(), 200);
    let records = sink.records.lock().unwrap().clone();
    let created = records
        .iter()
        .find(|record| record.message == "agent auth: registration.created")
        .unwrap_or_else(|| panic!("a registration.created record in {records:?}"));
    let field = |name: &str| {
        created
            .context
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    };
    assert_eq!(field("client_address").as_deref(), Some(PEER));
    assert_eq!(
        field("registration_id").as_deref(),
        registration.json()["registration_id"].as_str()
    );
    assert!(
        records
            .iter()
            .any(|record| record.message == "agent auth: assertion.issued")
    );
    let missing = Router::new()
        .try_agents(
            AgentsConfig::new()
                .oauth_server(OAuthServer::new().connection(world.db.clone()))
                .agent_auth(
                    AgentAuth::new()
                        .signing_key("k", service_pem())
                        .audit_log_channel(unique("no-such-channel")),
                ),
        )
        .err()
        .expect("an audit channel that does not exist fails the build");
    assert!(
        missing.to_string().contains("audit log channel"),
        "{missing}"
    );
}

/// Every audit name auth.md lists.
const AUDIT_NAMES: [&str; 9] = [
    "registration.created",
    "assertion.issued",
    "token.issued",
    "token.revoked",
    "claim.requested",
    "user_code.minted",
    "claim.confirmed",
    "registration.expired",
    "registration.revoked",
];

#[test]
fn agent_011_every_audit_event_is_recorded_with_the_client_address() {
    if !crate::own_process::is_child() {
        crate::own_process::run_alone(
            "agent_auth::agent_011_every_audit_event_is_recorded_with_the_client_address",
        );
        return;
    }
    // SAFETY: this test runs alone in its own process; nothing else reads
    // the environment meanwhile.
    unsafe {
        std::env::set_var("APP_ENV", "testing");
        std::env::set_var("APP_URL", ISSUER);
    }
    suprnova::boot::load_env().expect("pre-runtime configuration");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(async {
            let world =
                agent_world(|agent| agent.registration_window(Duration::from_secs(1))).await;
            let client = &world.client;
            let _events = EventFacade::fake_only(&["MagnetarLifecycleEvent"]);
            let user = new_account("audit");
            // Created, assertion issued, token issued and revoked.
            let registration = anonymous(client).await;
            let token =
                agent_token(client, registration["identity_assertion"].as_str().unwrap()).await;
            client
                .post("/oauth/revoke")
                .form(&[("token", token.as_str())])
                .send()
                .await
                .assert_ok();
            // Claim requested, user code minted, claim confirmed.
            let registration = service_auth(client, &account_email(&user)).await;
            sign_in(client, &user).await;
            let page = open_claim(
                client,
                registration["claim"]["verification_uri"].as_str().unwrap(),
            )
            .await;
            confirm(
                client,
                &page,
                registration["claim"]["user_code"].as_str().unwrap(),
            )
            .await
            .assert_ok();
            // Registration revoked, by the user.
            let id = registration["registration_id"].as_str().unwrap();
            let listed = client.get("/oauth/connected").send().await.body_text();
            client
                .post(format!("/oauth/connected/agents/{id}/revoke"))
                .form(&[("_token", form_token(&listed).as_str())])
                .send()
                .await;
            // Registration expired, by `agents:prune` after the window.
            let _unclaimed = anonymous(client).await;
            tokio::time::sleep(Duration::from_millis(1200)).await;
            let (db, pem, resolver, documents) = (
                world.db.clone(),
                world.pem.clone(),
                world.resolver.clone(),
                world.documents.clone(),
            );
            suprnova::Application::new()
                .routes(move || {
                    agent_router(&db, &pem, &resolver, &documents, |agent| {
                        agent.registration_window(Duration::from_secs(1))
                    })
                })
                .run_with_args(["app", "agents:prune"])
                .await
                .expect("agents:prune runs");

            let events = dispatched::<MagnetarLifecycleEvent>(|_| true);
            for name in AUDIT_NAMES {
                let named: Vec<&MagnetarLifecycleEvent> = events
                    .iter()
                    .filter(|event| event.kind.audit_name() == Some(name))
                    .collect();
                assert!(!named.is_empty(), "{name} was recorded: {events:?}");
                assert!(
                    named
                        .iter()
                        .all(|event| event.attributes.get("client_address")
                            == Some(&PEER.to_owned())),
                    "{name} carries the client address: {named:?}"
                );
            }
        });
}

// ---------------------------------------------------------- AGENT-010 ----
// The operator command, each in its own process: it boots as `serve` does.

#[test]
fn agent_010_operator_command_revokes_every_agent_credential() {
    if !crate::own_process::is_child() {
        crate::own_process::run_alone(
            "agent_auth::agent_010_operator_command_revokes_every_agent_credential",
        );
        return;
    }
    // SAFETY: this test runs alone in its own process.
    unsafe {
        std::env::set_var("APP_ENV", "testing");
        std::env::set_var("APP_URL", ISSUER);
    }
    suprnova::boot::load_env().expect("pre-runtime configuration");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(async {
            let world = agent_world(|agent| agent).await;
            let user = new_account("operator");
            let claimed = claim_anonymous(&world, &user).await;
            let bound = claimed.tokens["access_token"].as_str().unwrap().to_owned();
            let unclaimed = anonymous(&world.client).await;
            let pre_claim = agent_token(
                &world.client,
                unclaimed["identity_assertion"].as_str().unwrap(),
            )
            .await;
            let (db, pem, resolver, documents) = (
                world.db.clone(),
                world.pem.clone(),
                world.resolver.clone(),
                world.documents.clone(),
            );
            suprnova::Application::new()
                .routes(move || agent_router(&db, &pem, &resolver, &documents, |agent| agent))
                .run_with_args(["app", "agents:revoke-all", "--force"])
                .await
                .expect("agents:revoke-all runs");
            for token in [&bound, &pre_claim] {
                assert_eq!(
                    refusal(&world.client, token).await,
                    "The token was revoked."
                );
            }
            let again = exchange(
                &world.client,
                unclaimed["identity_assertion"].as_str().unwrap(),
                None,
            )
            .await;
            assert_eq!(again.json()["error"], "invalid_grant");
        });
}

#[test]
fn agent_010_operator_command_needs_force_in_production() {
    if !crate::own_process::is_child() {
        crate::own_process::run_alone(
            "agent_auth::agent_010_operator_command_needs_force_in_production",
        );
        return;
    }
    // SAFETY: this test runs alone in its own process.
    unsafe {
        std::env::set_var("APP_ENV", "production");
    }
    suprnova::boot::load_env().expect("pre-runtime configuration");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(async {
            let error = suprnova::Application::new()
                .run_with_args(["app", "agents:revoke-all"])
                .await
                .expect_err("production refuses without --force");
            assert!(error.message().contains("--force"), "{}", error.message());
            // `--force` from a test is not a terminal: still refused.
            let error = suprnova::Application::new()
                .run_with_args(["app", "agents:revoke-all", "--force"])
                .await
                .expect_err("production refuses without a typed confirmation");
            assert!(
                error.message().contains("not a terminal"),
                "{}",
                error.message()
            );
        });
}

// ---------------------------------------------------------- AGENT-017 ----

#[tokio::test]
async fn agent_017_default_claim_page_can_be_replaced() {
    let world = agent_world(|agent| agent.claim_page("/connect-agent")).await;
    let user = new_account("moved-page");
    let registration = service_auth(&world.client, &account_email(&user)).await;
    let uri = registration["claim"]["verification_uri"].as_str().unwrap();
    assert!(
        uri.starts_with("http://localhost/connect-agent?claim_attempt_token="),
        "{uri}"
    );
    sign_in(&world.client, &user).await;
    let page = open_claim(&world.client, uri).await;
    assert_eq!(page.status(), 200, "{}", page.body_text());
    assert!(
        page.body_text()
            .contains("action=\"http://localhost/connect-agent\"")
    );
    assert_eq!(world.client.get("/agent/claim").send().await.status(), 404);
}

#[tokio::test]
async fn agent_017_connected_page_lists_agents_beside_clients() {
    let world = agent_world(with_id_jags).await;
    let client = &world.client;
    let user = new_account("both");
    claim_anonymous(&world, &user).await;
    let page = client.get("/oauth/connected").send().await;
    assert_eq!(page.status(), 200);
    let text = page.body_text();
    assert!(text.contains("<h1>Connected apps</h1>"), "{text}");
    assert!(text.contains("No apps can act for you."), "{text}");
    assert!(text.contains("<h2>Agents</h2>"), "{text}");
    assert!(
        text.contains("Registered on its own and claimed by you"),
        "{text}"
    );
}

// ------------------------------------------- mixed-case stored email ----

/// A user of the Eloquent provider: the application's own `users` table.
#[suprnova::model(table = "users", fillable = ["email", "password"])]
pub struct EloquentAccount {
    pub id: i64,
    pub email: String,
    pub password: String,
    pub email_verified_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl suprnova::Authenticatable for EloquentAccount {
    fn get_auth_identifier(&self) -> String {
        self.id.to_string()
    }
    fn get_auth_password(&self) -> Option<&str> {
        Some(&self.password)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

impl suprnova::MustVerifyEmail for EloquentAccount {
    fn email(&self) -> &str {
        &self.email
    }
    fn email_verified_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.email_verified_at
    }
    fn set_email_verified_at(&mut self, at: Option<chrono::DateTime<chrono::Utc>>) {
        self.email_verified_at = at;
    }
}

impl suprnova::CanResetPassword for EloquentAccount {
    fn email_for_reset(&self) -> &str {
        &self.email
    }
    fn set_password_hash(&mut self, hash: &str) {
        self.password = hash.to_owned();
    }
}

/// A provisioner that counts what it is asked and creates nothing.
#[derive(Default)]
struct CountingProvisioner(std::sync::atomic::AtomicUsize);

#[async_trait]
impl AgentUserProvisioner for CountingProvisioner {
    async fn provision(
        &self,
        _identity: &ProvidedIdentity,
    ) -> Result<Option<String>, FrameworkError> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(None)
    }
}

#[tokio::test]
async fn agent_008_mixed_case_stored_email_matches_and_is_never_provisioned_again() {
    let provisioner = Arc::new(CountingProvisioner::default());
    let counted = provisioner.clone();
    let world = eloquent_world(&[("Alice@Corp.test", true)], move |agent| {
        with_id_jags(agent).provision_users(counted)
    })
    .await;
    let db = world.db.clone();
    let client = &world.client;

    let response = present(
        client,
        &world.id_jag(&id_jag_claims("alice-at-provider", "alice@corp.test")),
    )
    .await;
    assert_eq!(response.status(), 401, "{}", response.body_text());
    let body = response.json();
    assert_eq!(body["error"], "interaction_required", "the claim ceremony");
    sign_in(client, "1").await;
    let page = open_claim(client, body["claim"]["verification_uri"].as_str().unwrap()).await;
    assert_eq!(page.status(), 200, "{}", page.body_text());
    assert!(page.body_text().contains("Alice@Corp.test"));
    let done = confirm(client, &page, body["claim"]["user_code"].as_str().unwrap()).await;
    assert_eq!(done.status(), 200, "{}", done.body_text());
    forget_polls(&db).await;
    let tokens = poll(client, body["claim_token"].as_str().unwrap()).await;
    assert_eq!(tokens.status(), 200, "{}", tokens.body_text());
    let token = tokens.json()["access_token"].as_str().unwrap().to_owned();
    let (status, created) = call(client, &token, "posts.create", json!({ "title": "x" })).await;
    assert_eq!(status, 200, "{created}");
    assert_eq!(created["user"], "1");

    // The delegation is known now; a new subject with the same address
    // steps up again. Neither reaches the provisioner.
    let again = present(
        client,
        &world.id_jag(&id_jag_claims("alice-at-provider", "alice@corp.test")),
    )
    .await;
    assert_eq!(again.status(), 200, "{}", again.body_text());
    let other = present(
        client,
        &world.id_jag(&id_jag_claims("alice-second-login", "ALICE@corp.test")),
    )
    .await;
    assert_eq!(
        other.json()["error"],
        "interaction_required",
        "{}",
        other.body_text()
    );
    assert_eq!(
        provisioner.0.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "the stored account is found, never provisioned again"
    );
}

/// A world whose default guard reads the application's own `users` table
/// through the Eloquent provider, holding `accounts` (address, verified)
/// as ids 1, 2 and so on, on the same in-memory database as the OAuth
/// server and agent auth.
async fn eloquent_world(
    accounts: &[(&str, bool)],
    configure: impl FnOnce(AgentAuth) -> AgentAuth,
) -> AgentWorld {
    let container = crate::oauth::container_with_users(Arc::new(suprnova::EloquentUserProvider::<
        EloquentAccount,
    >::new()));
    let connection = suprnova::DbConnection::connect(
        &suprnova::DatabaseConfig::builder()
            .url("sqlite::memory:")
            .max_connections(1)
            .min_connections(1)
            .build(),
    )
    .await
    .unwrap();
    suprnova::container::testing::TestContainer::singleton(connection.clone());
    let db = connection.inner().clone();
    db.execute_unprepared(
        "CREATE TABLE users (id INTEGER PRIMARY KEY AUTOINCREMENT, email TEXT NOT NULL, \
         password TEXT NOT NULL, email_verified_at TEXT)",
    )
    .await
    .unwrap();
    for (email, verified) in accounts {
        let verified_at = if *verified {
            "'2026-01-01T00:00:00Z'"
        } else {
            "NULL"
        };
        db.execute_unprepared(&format!(
            "INSERT INTO users (email, password, email_verified_at) \
             VALUES ('{email}', 'stored-hash', {verified_at})"
        ))
        .await
        .unwrap();
    }
    OAuthServer::migrate(&db).await.unwrap();
    AgentAuth::migrate(&db).await.unwrap();
    agent_world_in(container, db, configure)
}

/// A provisioner that finds a verified account with the provider's address
/// whatever its case, and creates one only when none exists, counting what
/// it is asked. It breaks [`AgentUserProvisioner`]'s contract on purpose:
/// it returns an existing user.
struct FindOrCreate {
    db: DatabaseConnection,
    asked: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl AgentUserProvisioner for FindOrCreate {
    async fn provision(
        &self,
        identity: &ProvidedIdentity,
    ) -> Result<Option<String>, FrameworkError> {
        self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let Some(email) = identity.verified_email.clone() else {
            return Ok(None);
        };
        let found = self
            .db
            .query_one_raw(sea_orm::Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT id FROM users WHERE LOWER(email) = LOWER(?) \
                 AND email_verified_at IS NOT NULL",
                [email.into()],
            ))
            .await
            .map_err(|error| FrameworkError::internal(error.to_string()))?;
        Ok(found
            .and_then(|row| row.try_get::<i64>("", "id").ok())
            .map(|id| id.to_string()))
    }
}

#[tokio::test]
async fn agent_008_unverified_exact_case_row_does_not_hide_the_verified_account() {
    // Someone signed up as alice@corp.test and never verified it; Alice's
    // verified account is Alice@Corp.test.
    let accounts = [("Alice@Corp.test", true), ("alice@corp.test", false)];
    let refusing = eloquent_world(&accounts, with_id_jags).await;
    let response = present(
        &refusing.client,
        &refusing.id_jag(&id_jag_claims("alice-default", "alice@corp.test")),
    )
    .await;
    assert_eq!(response.status(), 401, "{}", response.body_text());
    assert_eq!(response.json()["error"], "interaction_required");
    drop(refusing);

    let world = eloquent_world(&accounts, |agent| agent).await;
    let finder = Arc::new(FindOrCreate {
        db: world.db.clone(),
        asked: std::sync::atomic::AtomicUsize::new(0),
    });
    let provisioning = world.node(|agent| with_id_jags(agent).provision_users(finder.clone()));
    let response = present(
        &provisioning,
        &world.id_jag(&id_jag_claims("alice-finder", "alice@corp.test")),
    )
    .await;
    assert_eq!(response.status(), 401, "{}", response.body_text());
    let body = response.json();
    assert_eq!(
        body["error"], "interaction_required",
        "the claim ceremony, never a silent bind"
    );
    assert!(body.get("identity_assertion").is_none());
    assert_eq!(finder.asked.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn agent_008_two_verified_accounts_differing_in_case_are_refused() {
    let provisioner = Arc::new(CountingProvisioner::default());
    let counted = provisioner.clone();
    let world = eloquent_world(
        &[("Bob@Corp.test", true), ("bob@corp.test", true)],
        move |agent| with_id_jags(agent).provision_users(counted),
    )
    .await;
    let refused = present(
        &world.client,
        &world.id_jag(&id_jag_claims("bob-at-provider", "bob@corp.test")),
    )
    .await;
    assert_eq!(refused.status(), 400, "{}", refused.body_text());
    let body = refused.json();
    assert_eq!(body["error"], "invalid_request");
    assert!(
        body["error_description"]
            .as_str()
            .is_some_and(|text| text.contains("more than one account")),
        "{body}"
    );
    assert_eq!(provisioner.0.load(std::sync::atomic::Ordering::SeqCst), 0);
}
