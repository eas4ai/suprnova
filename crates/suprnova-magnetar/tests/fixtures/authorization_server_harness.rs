//! Composition harness for the authorization server suites (AGENT-005).
//!
//! Builds the engine and its plugin over Magnetar's default schema, so the
//! same harness runs on in-memory SQLite and on a live PostgreSQL or MySQL
//! database. Client metadata documents come from a scripted resolver and a
//! scripted pinned client; nothing reaches the network.

#![allow(dead_code)]

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use magnetar::auth::{AuthenticationContext, FactorGate, SignInDecision, VerifiedPrincipal};
use magnetar::authorization_server::{
    AccessVerification, AuthorizationServer, AuthorizationServerConfig, AuthorizationStore,
    ClientMetadataFetcher, ClientMetadataPolicy, GrantHandler, HostResolver, InvalidAccessReason,
    PinnedHttpClient, PinnedResponse, ResponseBody, ScopeDefinition, SqlAuthorizationStore,
};
use magnetar::default_schema::DefaultAuthSchema;
use magnetar::default_schema::sql_stores::SqlSessionStore;
use magnetar::plugin::{
    BearerCredential, Effect, Encryptor, HttpRequest, HttpResponse, HttpTransport, Method,
    PluginContext, PluginRegistry, WireBody, WireRequest, WireResponse,
};
use magnetar::plugins::authorization_server::AuthorizationServerPlugin;
use magnetar::sessions::{
    OpaqueConfig, OpaqueSessionProvider, OpaqueSessionStore, SessionGrant, SessionMetadata,
    StoredSession,
};
use magnetar::storage::{NewUser, SeaOrmStorage, UserStore};
use magnetar::{Error, Result};
use parking_lot::Mutex;
use sea_orm::DatabaseConnection;
use serde_json::Value;
use sha2::{Digest, Sha256};
use url::Url;

#[path = "fakes.rs"]
mod fakes;
#[allow(unused_imports)]
pub use fakes::{CountingLimiter, LimiterMode, RecordingMail, TestLinks};

pub const ISSUER: &str = "https://auth.example.test";
pub const RESOURCE: &str = "https://app.example.test/mcp";
pub const REDIRECT: &str = "https://client.example.test/callback";
pub const DOCUMENT_CLIENT: &str = "https://tools.client.test/oauth/client.json";
pub const DOCUMENT_HOST: &str = "tools.client.test";
pub const DOCUMENT_REDIRECT: &str = "https://tools.client.test/callback";
pub const PUBLIC_ADDRESS: &str = "93.184.215.14";

/// The configuration every suite starts from: two declared scopes.
pub fn config() -> AuthorizationServerConfig {
    let mut config = AuthorizationServerConfig::new(ISSUER, vec![RESOURCE.to_owned()]);
    config.scopes = vec![
        ScopeDefinition::new("tools:read", "Read your tools"),
        ScopeDefinition::new("tools:write", "Change data through tools"),
    ];
    config
}

struct IdentityEncryptor;

#[async_trait]
impl Encryptor for IdentityEncryptor {
    async fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        Ok(plaintext.to_vec())
    }

    async fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        Ok(ciphertext.to_vec())
    }
}

/// The plugin never signs anyone in, so the gate only has to exist.
struct UnusedFactorGate;

#[async_trait]
impl FactorGate for UnusedFactorGate {
    async fn complete_sign_in(
        &self,
        _principal: VerifiedPrincipal,
        _context: AuthenticationContext,
    ) -> Result<SignInDecision> {
        Err(Error::Internal {
            message: "the authorization server suite signs no one in".to_owned(),
        })
    }

    async fn complete_challenge(&self, _selector: &str, _code: &str) -> Result<SessionGrant> {
        Err(Error::Internal {
            message: "the authorization server suite signs no one in".to_owned(),
        })
    }
}

struct NoHttp;

#[async_trait]
impl HttpTransport for NoHttp {
    async fn send(&self, _request: HttpRequest) -> Result<HttpResponse> {
        Err(Error::Internal {
            message: "the authorization server sends nothing through the host transport".to_owned(),
        })
    }
}

/// A resolver that answers from a table and records every lookup.
#[derive(Default)]
pub struct FakeResolver {
    answers: Mutex<HashMap<String, Vec<IpAddr>>>,
    pub lookups: Mutex<Vec<String>>,
}

impl FakeResolver {
    pub fn answer(&self, host: &str, addresses: &[&str]) {
        self.answers.lock().insert(
            host.to_owned(),
            addresses
                .iter()
                .map(|address| address.parse().expect("a test address"))
                .collect(),
        );
    }

    pub fn lookup_count(&self) -> usize {
        self.lookups.lock().len()
    }
}

#[async_trait]
impl HostResolver for FakeResolver {
    async fn resolve(&self, host: &str, _port: u16) -> Result<Vec<IpAddr>> {
        self.lookups.lock().push(host.to_owned());
        self.answers
            .lock()
            .get(host)
            .cloned()
            .ok_or_else(|| Error::NotFound {
                resource: "host".to_owned(),
                identifier: host.to_owned(),
            })
    }
}

/// One scripted document response.
#[derive(Clone)]
pub struct Script {
    pub status: u16,
    pub body: Vec<u8>,
    pub delay: Duration,
    pub endless: bool,
}

impl Script {
    pub fn json(body: Value) -> Self {
        Self {
            status: 200,
            body: serde_json::to_vec(&body).expect("a JSON body"),
            delay: Duration::ZERO,
            endless: false,
        }
    }
}

/// A pinned client that serves scripted documents and records which
/// address each request was told to connect to.
#[derive(Default)]
pub struct FakeDocuments {
    scripts: Mutex<HashMap<String, Script>>,
    pub requests: Mutex<Vec<(String, SocketAddr)>>,
}

impl FakeDocuments {
    pub fn serve(&self, url: &str, script: Script) {
        self.scripts.lock().insert(url.to_owned(), script);
    }

    pub fn request_count(&self) -> usize {
        self.requests.lock().len()
    }
}

struct ScriptBody {
    remaining: Vec<u8>,
    endless: bool,
}

#[async_trait]
impl ResponseBody for ScriptBody {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>> {
        if self.endless {
            return Ok(Some(vec![b' '; 8192]));
        }
        if self.remaining.is_empty() {
            return Ok(None);
        }
        let take = self.remaining.len().min(4096);
        Ok(Some(self.remaining.drain(..take).collect()))
    }
}

#[async_trait]
impl PinnedHttpClient for FakeDocuments {
    async fn get(&self, url: &Url, address: SocketAddr) -> Result<PinnedResponse> {
        self.requests.lock().push((url.to_string(), address));
        let script = self.scripts.lock().get(url.as_str()).cloned();
        let Some(script) = script else {
            return Ok(PinnedResponse {
                status: 404,
                cache_control: None,
                body: Box::new(ScriptBody {
                    remaining: Vec::new(),
                    endless: false,
                }),
            });
        };
        if !script.delay.is_zero() {
            tokio::time::sleep(script.delay).await;
        }
        Ok(PinnedResponse {
            status: script.status,
            cache_control: None,
            body: Box::new(ScriptBody {
                remaining: script.body,
                endless: script.endless,
            }),
        })
    }
}

/// The composed world one suite runs in.
pub struct Harness {
    pub db: DatabaseConnection,
    pub storage: Arc<SeaOrmStorage<DefaultAuthSchema>>,
    pub server: Arc<AuthorizationServer>,
    pub registry: PluginRegistry<DefaultAuthSchema>,
    pub resolver: Arc<FakeResolver>,
    pub documents: Arc<FakeDocuments>,
    pub limiter: Arc<CountingLimiter>,
    pub fetcher: Arc<ClientMetadataFetcher>,
}

/// The default harness over in-memory SQLite.
pub async fn sqlite_harness() -> Harness {
    build(
        magnetar::default_schema::database().await,
        config(),
        ClientMetadataPolicy::default(),
        Vec::new(),
    )
    .await
}

/// A harness over a live database at `url`, after the default migration.
pub async fn live_harness(url: &str) -> Harness {
    let db = sea_orm::Database::connect(url)
        .await
        .expect("connect the live backend");
    magnetar::default_schema::migrate(&db)
        .await
        .expect("create the default auth tables and the authorization server's");
    build(db, config(), ClientMetadataPolicy::default(), Vec::new()).await
}

pub async fn build(
    db: DatabaseConnection,
    config: AuthorizationServerConfig,
    policy: ClientMetadataPolicy,
    handlers: Vec<Arc<dyn GrantHandler>>,
) -> Harness {
    let store = Arc::new(SqlAuthorizationStore::new(db.clone()));
    build_with_store(db, config, policy, handlers, store).await
}

/// [`build`] over a store the test supplies, such as one that fails on
/// purpose.
pub async fn build_with_store(
    db: DatabaseConnection,
    config: AuthorizationServerConfig,
    policy: ClientMetadataPolicy,
    handlers: Vec<Arc<dyn GrantHandler>>,
    store: Arc<dyn AuthorizationStore>,
) -> Harness {
    let storage = Arc::new(SeaOrmStorage::<DefaultAuthSchema>::new(db.clone()));
    let resolver = Arc::new(FakeResolver::default());
    let documents = Arc::new(FakeDocuments::default());
    let limiter = Arc::new(CountingLimiter::default());
    let fetcher = Arc::new(ClientMetadataFetcher::new(
        resolver.clone(),
        documents.clone(),
        policy,
    ));
    let mut server = AuthorizationServer::new(
        config,
        store,
        storage.clone(),
        limiter.clone(),
        fetcher.clone(),
    )
    .expect("a valid authorization server configuration");
    for handler in handlers {
        server = server
            .with_grant_handler(handler)
            .expect("an extension grant type");
    }
    let server = Arc::new(server);
    let sessions = Arc::new(OpaqueSessionProvider::new(
        Arc::new(SqlSessionStore(db.clone())),
        OpaqueConfig::default(),
    ));
    let context: PluginContext<DefaultAuthSchema> = PluginContext::new(
        storage.clone(),
        sessions,
        Arc::new(UnusedFactorGate),
        Arc::new(IdentityEncryptor),
        limiter.clone(),
        Arc::new(RecordingMail::default()),
        Arc::new(NoHttp),
        Arc::new(TestLinks),
    );
    let registry = PluginRegistry::new(context)
        .register(AuthorizationServerPlugin::new(server.clone()))
        .build()
        .await
        .expect("the authorization server plugin mounts");
    Harness {
        db,
        storage,
        server,
        registry,
        resolver,
        documents,
        limiter,
        fetcher,
    }
}

/// A fresh PKCE verifier and its S256 challenge.
pub fn pkce() -> (String, String) {
    let verifier = format!(
        "{:016x}{:016x}{:016x}{:016x}",
        rand::random::<u64>(),
        rand::random::<u64>(),
        rand::random::<u64>(),
        rand::random::<u64>()
    );
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

pub fn unique(label: &str) -> String {
    format!("{label}-{:016x}", rand::random::<u64>())
}

pub fn status(response: &WireResponse) -> u16 {
    response.0.status
}

pub fn location(response: &WireResponse) -> Option<String> {
    response.effects().iter().find_map(|effect| match effect {
        Effect::Redirect { location, .. } => Some(location.clone()),
        _ => None,
    })
}

pub fn header(response: &WireResponse, name: &str) -> Option<String> {
    response.0.headers.get(name).cloned()
}

pub fn body(response: &WireResponse) -> Value {
    response.0.body.clone().unwrap_or(Value::Null)
}

/// A query parameter of an absolute or host-relative address.
pub fn query_param(address: &str, name: &str) -> Option<String> {
    let url = Url::parse("https://host.invalid")
        .expect("a base")
        .join(address)
        .expect("an address");
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

pub fn assert_no_framing(response: &WireResponse) {
    assert_eq!(header(response, "x-frame-options").as_deref(), Some("DENY"));
    assert_eq!(
        header(response, "content-security-policy").as_deref(),
        Some("frame-ancestors 'none'")
    );
}

/// The tokens one completed code flow produced.
pub struct Issued {
    pub user_id: String,
    pub bearer: String,
    pub client_id: String,
    pub access_token: String,
    pub refresh_token: String,
}

impl Harness {
    pub async fn send(&self, request: WireRequest, bearer: Option<&str>) -> WireResponse {
        self.registry
            .handle_bound(request, bearer.map(BearerCredential::new))
            .await
            .expect("the plugin answers")
    }

    pub async fn get(
        &self,
        path: &str,
        query: &[(&str, &str)],
        bearer: Option<&str>,
    ) -> WireResponse {
        let mut request = WireRequest::new(Method::Get, path);
        request.query = query
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        request
            .headers
            .insert("x-client-ip".to_owned(), "198.51.100.7".to_owned());
        self.send(request, bearer).await
    }

    pub async fn post_form(
        &self,
        path: &str,
        fields: &[(&str, &str)],
        bearer: Option<&str>,
    ) -> WireResponse {
        let mut request = WireRequest::new(Method::Post, path);
        request.body = WireBody::Form(
            fields
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
        );
        request
            .headers
            .insert("x-client-ip".to_owned(), "198.51.100.7".to_owned());
        self.send(request, bearer).await
    }

    pub async fn post_json(&self, path: &str, body: Value) -> WireResponse {
        let mut request = WireRequest::new(Method::Post, path);
        request.body = WireBody::Json(body);
        request
            .headers
            .insert("x-client-ip".to_owned(), "198.51.100.7".to_owned());
        self.send(request, None).await
    }

    pub async fn create_user(&self) -> String {
        self.storage
            .create_user(NewUser {
                email: format!("{}@example.test", unique("agent-005")),
                password_hash: Some("fixture-hash".to_owned()),
            })
            .await
            .expect("create a user")
            .user_id
    }

    /// Sign the user in at their current epoch and return the bearer.
    pub async fn sign_in(&self, user_id: &str) -> String {
        let user = self
            .storage
            .find_by_id(user_id)
            .await
            .expect("read the user")
            .expect("the user exists");
        let token = unique("session-token");
        let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        SqlSessionStore(self.db.clone())
            .insert_session_if_epoch_current(StoredSession {
                session_id: unique("session"),
                user_id: user_id.to_owned(),
                auth_epoch: user.auth_epoch,
                token_hash: digest,
                token_digest: digest,
                expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
                revoked_at: None,
                metadata: SessionMetadata::default(),
            })
            .await
            .expect("store the session");
        token
    }

    pub async fn register(&self, redirect_uris: &[&str]) -> String {
        let response = self
            .post_json(
                "/oauth/register",
                serde_json::json!({
                    "client_name": "Example Tool",
                    "redirect_uris": redirect_uris,
                    "token_endpoint_auth_method": "none",
                    "grant_types": ["authorization_code", "refresh_token"],
                    "response_types": ["code"],
                }),
            )
            .await;
        assert_eq!(status(&response), 201, "{:?}", body(&response));
        body(&response)["client_id"]
            .as_str()
            .expect("a client_id")
            .to_owned()
    }

    pub async fn authorize(
        &self,
        client_id: &str,
        redirect_uri: &str,
        challenge: &str,
        scope: &str,
        bearer: Option<&str>,
    ) -> WireResponse {
        self.get(
            "/oauth/authorize",
            &[
                ("response_type", "code"),
                ("client_id", client_id),
                ("redirect_uri", redirect_uri),
                ("code_challenge", challenge),
                ("code_challenge_method", "S256"),
                ("scope", scope),
                ("resource", RESOURCE),
                ("state", "state-123"),
            ],
            bearer,
        )
        .await
    }

    /// Authorize and approve; return the code.
    pub async fn approve(
        &self,
        client_id: &str,
        redirect_uri: &str,
        challenge: &str,
        scope: &str,
        bearer: &str,
    ) -> String {
        let response = self
            .authorize(client_id, redirect_uri, challenge, scope, Some(bearer))
            .await;
        let approval = location(&response).unwrap_or_else(|| {
            panic!(
                "a redirect to the approval page, got {} {:?}",
                status(&response),
                body(&response)
            )
        });
        let id = query_param(&approval, "authorization").expect("a pending authorization");
        let decided = self
            .post_form(
                &format!("/oauth/authorizations/{id}"),
                &[("decision", "approve")],
                Some(bearer),
            )
            .await;
        let back = location(&decided).unwrap_or_else(|| {
            panic!(
                "a redirect back to the client, got {} {:?}",
                status(&decided),
                body(&decided)
            )
        });
        query_param(&back, "code").expect("a code")
    }

    pub async fn exchange(
        &self,
        client_id: &str,
        redirect_uri: &str,
        code: &str,
        verifier: &str,
    ) -> WireResponse {
        self.post_form(
            "/oauth/token",
            &[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", redirect_uri),
                ("client_id", client_id),
                ("code_verifier", verifier),
                ("resource", RESOURCE),
            ],
            None,
        )
        .await
    }

    pub async fn refresh(&self, client_id: &str, refresh_token: &str) -> WireResponse {
        self.post_form(
            "/oauth/token",
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", client_id),
            ],
            None,
        )
        .await
    }

    pub async fn verify(&self, token: &str) -> AccessVerification {
        self.server
            .verify_access_token(token, RESOURCE)
            .await
            .expect("verification reads storage")
    }

    /// Register a client, sign a new user in and complete the code flow.
    pub async fn issue(&self, scope: &str) -> Issued {
        let user_id = self.create_user().await;
        let bearer = self.sign_in(&user_id).await;
        let client_id = self.register(&[REDIRECT]).await;
        let (verifier, challenge) = pkce();
        let code = self
            .approve(&client_id, REDIRECT, &challenge, scope, &bearer)
            .await;
        let response = self.exchange(&client_id, REDIRECT, &code, &verifier).await;
        assert_eq!(status(&response), 200, "{:?}", body(&response));
        let tokens = body(&response);
        Issued {
            user_id,
            bearer,
            client_id,
            access_token: tokens["access_token"].as_str().expect("access").to_owned(),
            refresh_token: tokens["refresh_token"]
                .as_str()
                .expect("refresh")
                .to_owned(),
        }
    }
}

/// Present one refresh token from many tasks at once: at most one wins,
/// and afterwards no token of the grant works.
pub async fn concurrent_refresh_reuse(h: Arc<Harness>) {
    let issued = h.issue("tools:read").await;
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let h = h.clone();
        let client_id = issued.client_id.clone();
        let refresh = issued.refresh_token.clone();
        tasks.push(tokio::spawn(async move {
            let response = h.refresh(&client_id, &refresh).await;
            (status(&response), body(&response))
        }));
    }
    let mut winners = Vec::new();
    for task in tasks {
        let (code, answer) = task.await.expect("a refresh task");
        if code == 200 {
            winners.push(answer);
        } else {
            assert_eq!(answer["error"], "invalid_grant", "{answer:?}");
        }
    }
    assert!(winners.len() <= 1, "{} refreshes won", winners.len());
    let revoked = AccessVerification::Invalid(InvalidAccessReason::Revoked);
    assert_eq!(h.verify(&issued.access_token).await, revoked);
    for winner in winners {
        assert_eq!(
            h.verify(winner["access_token"].as_str().expect("access"))
                .await,
            revoked
        );
        let again = h
            .refresh(
                &issued.client_id,
                winner["refresh_token"].as_str().expect("refresh"),
            )
            .await;
        assert_eq!(body(&again)["error"], "invalid_grant");
    }
}

/// The rotation guard holds in the database itself: with the service's own
/// read out of the way, a second rotation of one refresh token affects no
/// row and inserts nothing.
pub async fn guarded_rotation(h: &Harness) {
    use magnetar::authorization_server::{
        AuthorizationStore, NewTokenRecord, REFRESH_TOKEN_KIND, SqlAuthorizationStore,
    };

    let issued = h.issue("tools:read").await;
    let store = SqlAuthorizationStore::new(h.db.clone());
    let digest = format!("{:x}", Sha256::digest(issued.refresh_token.as_bytes()));
    let token = store
        .find_token(&digest)
        .await
        .expect("read the refresh token")
        .expect("the refresh token is stored");
    let replacement = |label: &str| NewTokenRecord {
        id: unique(label),
        kind: REFRESH_TOKEN_KIND.to_owned(),
        token_hash: format!("{:x}", Sha256::digest(unique(label).as_bytes())),
        scopes: vec!["tools:read".to_owned()],
        expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
    };
    let now = chrono::Utc::now();
    assert!(
        store
            .rotate_refresh_token(&token.id, &token.grant_id, now, vec![replacement("first")])
            .await
            .expect("the first rotation")
    );
    let second = replacement("second");
    assert!(
        !store
            .rotate_refresh_token(&token.id, &token.grant_id, now, vec![second.clone()])
            .await
            .expect("the second rotation")
    );
    assert_eq!(
        store.find_token(&second.token_hash).await.expect("read"),
        None,
        "a losing rotation inserts nothing"
    );
}

/// The code's single-winner step holds in the database itself: with the
/// service's own read out of the way, a second redemption of one code
/// moves nothing.
pub async fn guarded_redemption(h: &Harness) {
    use magnetar::authorization_server::{AuthorizationRecord, SqlAuthorizationStore};

    let store = SqlAuthorizationStore::new(h.db.clone());
    let now = chrono::Utc::now();
    let id = unique("code-digest");
    store
        .create_authorization(AuthorizationRecord {
            id: id.clone(),
            kind: "code".to_owned(),
            state: "issued".to_owned(),
            user_id: "guard-user".to_owned(),
            payload: "{}".to_owned(),
            expires_at: now + chrono::Duration::minutes(5),
            created_at: now,
        })
        .await
        .expect("store a code");
    assert!(
        store
            .transition_authorization(&id, "code", "issued", "redeeming:first", Some(now))
            .await
            .expect("the first redemption")
    );
    assert!(
        !store
            .transition_authorization(&id, "code", "issued", "redeeming:second", Some(now))
            .await
            .expect("the second redemption")
    );
    let record = store
        .find_authorization(&id, "code", now)
        .await
        .expect("read the code")
        .expect("the code is still there");
    assert_eq!(record.state, "redeeming:first", "the loser moved nothing");
}

/// A decision spends its pending authorization once, in the database.
pub async fn guarded_decision(h: &Harness) {
    use magnetar::authorization_server::{AuthorizationRecord, SqlAuthorizationStore};

    let store = SqlAuthorizationStore::new(h.db.clone());
    let now = chrono::Utc::now();
    let id = unique("pending");
    store
        .create_authorization(AuthorizationRecord {
            id: id.clone(),
            kind: "pending".to_owned(),
            state: "pending".to_owned(),
            user_id: "guard-user".to_owned(),
            payload: "{}".to_owned(),
            expires_at: now + chrono::Duration::minutes(5),
            created_at: now,
        })
        .await
        .expect("store a pending authorization");
    assert!(
        store
            .consume_authorization(&id, "pending", now)
            .await
            .expect("the first decision")
    );
    assert!(
        !store
            .consume_authorization(&id, "pending", now)
            .await
            .expect("the second decision")
    );
}

/// A rotation and a revocation of one grant, queued behind a lock on the
/// grant row with the revocation first, both finish without a deadlock and
/// leave no token of the grant working. A rotation that took the token row
/// before the grant row deadlocks here: the revocation holds the grant and
/// waits for the token, while the rotation holds the token and waits for
/// the grant. PostgreSQL and MySQL only; SQLite has no row locks.
pub async fn revocation_and_rotation_take_locks_in_one_order(h: &Harness) {
    use magnetar::authorization_server::{
        NewTokenRecord, REFRESH_TOKEN_KIND, SqlAuthorizationStore,
    };
    use sea_orm::{ConnectionTrait, Statement, TransactionTrait};

    let issued = h.issue("tools:read").await;
    let store = SqlAuthorizationStore::new(h.db.clone());
    let digest = format!("{:x}", Sha256::digest(issued.refresh_token.as_bytes()));
    let token = store
        .find_token(&digest)
        .await
        .expect("read the refresh token")
        .expect("the refresh token is stored");

    let backend = h.db.get_database_backend();
    let placeholder = if backend == sea_orm::DbBackend::Postgres {
        "$1"
    } else {
        "?"
    };
    let holder = h.db.begin().await.expect("a transaction");
    holder
        .execute_raw(Statement::from_sql_and_values(
            backend,
            format!("SELECT id FROM auth_oauth_grants WHERE id = {placeholder} FOR UPDATE"),
            [token.grant_id.clone().into()],
        ))
        .await
        .expect("lock the grant row");

    let revocation = tokio::spawn({
        let store = store.clone();
        let grant = token.grant_id.clone();
        async move { store.revoke_grant(&grant, chrono::Utc::now()).await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    let rotation = tokio::spawn({
        let store = store.clone();
        let token = token.clone();
        async move {
            let replacement = NewTokenRecord {
                id: unique("replacement"),
                kind: REFRESH_TOKEN_KIND.to_owned(),
                token_hash: format!("{:x}", Sha256::digest(unique("replacement").as_bytes())),
                scopes: vec!["tools:read".to_owned()],
                expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            };
            store
                .rotate_refresh_token(
                    &token.id,
                    &token.grant_id,
                    chrono::Utc::now(),
                    vec![replacement],
                )
                .await
        }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    holder.commit().await.expect("release the grant row");

    let revoked = revocation.await.expect("the revocation task");
    let rotated = rotation.await.expect("the rotation task");
    assert!(revoked.is_ok(), "the revocation failed: {revoked:?}");
    assert!(rotated.is_ok(), "the rotation failed: {rotated:?}");
    assert!(
        !rotated.unwrap(),
        "the rotation must lose to the earlier revocation"
    );
    assert_eq!(
        h.verify(&issued.access_token).await,
        AccessVerification::Invalid(InvalidAccessReason::Revoked)
    );
    let refreshed = h.refresh(&issued.client_id, &issued.refresh_token).await;
    assert_eq!(body(&refreshed)["error"], "invalid_grant");
}

/// Trimming keeps the newest steps of one user and one kind, the step just
/// created always among them, even when every step was created in the same
/// second (MySQL keeps whole seconds); other users and kinds are untouched.
pub async fn trimming_is_scoped_and_keeps_the_new_step(h: &Harness) {
    use magnetar::authorization_server::sql::authorizations;
    use magnetar::authorization_server::{AuthorizationRecord, SqlAuthorizationStore};
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let store = SqlAuthorizationStore::new(h.db.clone());
    let second = chrono::DateTime::from_timestamp(chrono::Utc::now().timestamp(), 0)
        .expect("a whole second");
    let (user, other_user) = (unique("trim-user"), unique("trim-other"));
    let record = |id: String, user: &str, kind: &str| AuthorizationRecord {
        id,
        kind: kind.to_owned(),
        state: "pending".to_owned(),
        user_id: user.to_owned(),
        payload: "{}".to_owned(),
        expires_at: second + chrono::Duration::minutes(10),
        created_at: second,
    };
    for index in 0..6 {
        store
            .create_authorization(record(unique(&format!("trim-{index}")), &user, "pending"))
            .await
            .expect("a pending step");
    }
    for index in 0..3 {
        store
            .create_authorization(record(
                unique(&format!("other-{index}")),
                &other_user,
                "pending",
            ))
            .await
            .expect("another user's step");
        store
            .create_authorization(record(unique(&format!("code-{index}")), &user, "code"))
            .await
            .expect("a step of another kind");
    }
    // Sorts first by identifier, so a tie broken by identifier alone would
    // drop it.
    let newest = format!("0000-{}", unique("newest"));
    store
        .create_authorization(record(newest.clone(), &user, "pending"))
        .await
        .expect("the new step");

    let removed = store
        .trim_authorizations(&user, "pending", 3, &newest)
        .await
        .expect("trim");
    assert_eq!(removed, 4, "seven pending steps trimmed to three");
    let count = |user: String, kind: &'static str| {
        let db = h.db.clone();
        async move {
            authorizations::Entity::find()
                .filter(authorizations::Column::UserId.eq(user))
                .filter(authorizations::Column::Kind.eq(kind))
                .all(&db)
                .await
                .expect("read steps")
        }
    };
    let kept = count(user.clone(), "pending").await;
    assert_eq!(kept.len(), 3);
    assert!(
        kept.iter().any(|step| step.id == newest),
        "the new step is kept"
    );
    assert_eq!(
        count(other_user, "pending").await.len(),
        3,
        "another user is untouched"
    );
    assert_eq!(
        count(user, "code").await.len(),
        3,
        "another kind is untouched"
    );
}

/// The step that finishes a redemption moves a code even after its row
/// lapsed; the checked form refuses it.
pub async fn the_final_code_step_ignores_expiry(h: &Harness) {
    use magnetar::authorization_server::{AuthorizationRecord, SqlAuthorizationStore};

    let store = SqlAuthorizationStore::new(h.db.clone());
    let now = chrono::Utc::now();
    let id = unique("lapsed-code");
    store
        .create_authorization(AuthorizationRecord {
            id: id.clone(),
            kind: "code".to_owned(),
            state: "redeeming:grant".to_owned(),
            user_id: "final-step-user".to_owned(),
            payload: "{}".to_owned(),
            expires_at: now - chrono::Duration::seconds(5),
            created_at: now - chrono::Duration::minutes(10),
        })
        .await
        .expect("a lapsed code");
    assert!(
        !store
            .transition_authorization(&id, "code", "redeeming:grant", "redeemed:grant", Some(now))
            .await
            .expect("the checked step")
    );
    assert!(
        store
            .transition_authorization(&id, "code", "redeeming:grant", "redeemed:grant", None)
            .await
            .expect("the final step")
    );
}

/// Pruning keeps every grant inside the retention and drops one idle past
/// it. Each kept grant sits on one side of one condition, so removing
/// `revoked_at < cutoff`, `created_at < cutoff` or the `last_used_at`
/// condition deletes a grant this keeps.
pub async fn pruning_respects_the_retention_boundaries(h: &Harness) {
    use magnetar::authorization_server::sql::{grants, tokens};
    use sea_orm::sea_query::Expr;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let now = chrono::Utc::now();
    let recently = now - chrono::Duration::days(29);
    let long_ago = now - chrono::Duration::days(31);
    let mut ids = Vec::new();
    for _ in 0..4 {
        let issued = h.issue("tools:read").await;
        ids.push(
            h.server.list_grants(&issued.user_id).await.expect("list")[0]
                .grant_id
                .clone(),
        );
    }
    let [revoked_recently, young_idle, used_recently, long_idle] =
        <[String; 4]>::try_from(ids).expect("four grants");
    let set = |id: &str, column: grants::Column, value: chrono::DateTime<chrono::Utc>| {
        let db = h.db.clone();
        let id = id.to_owned();
        async move {
            grants::Entity::update_many()
                .col_expr(column, Expr::value(value))
                .filter(grants::Column::Id.eq(id))
                .exec(&db)
                .await
                .expect("set a grant column");
        }
    };
    let drop_tokens = |id: &str| {
        let db = h.db.clone();
        let id = id.to_owned();
        async move {
            tokens::Entity::delete_many()
                .filter(tokens::Column::GrantId.eq(id))
                .exec(&db)
                .await
                .expect("drop a grant's tokens");
        }
    };
    // Revoked 29 days ago, created long ago: kept only by `revoked_at`.
    drop_tokens(&revoked_recently).await;
    set(&revoked_recently, grants::Column::RevokedAt, recently).await;
    set(&revoked_recently, grants::Column::CreatedAt, long_ago).await;
    set(&revoked_recently, grants::Column::LastUsedAt, long_ago).await;
    // No tokens, created 29 days ago, never used: kept only by `created_at`.
    drop_tokens(&young_idle).await;
    set(&young_idle, grants::Column::CreatedAt, recently).await;
    // No tokens, created 31 days ago, used 29 days ago: kept only by
    // `last_used_at`.
    drop_tokens(&used_recently).await;
    set(&used_recently, grants::Column::CreatedAt, long_ago).await;
    set(&used_recently, grants::Column::LastUsedAt, recently).await;
    // No tokens, created and last used 31 days ago: dropped.
    drop_tokens(&long_idle).await;
    set(&long_idle, grants::Column::CreatedAt, long_ago).await;
    set(&long_idle, grants::Column::LastUsedAt, long_ago).await;

    h.server.prune_expired().await.expect("prune");
    let remaining: Vec<String> = grants::Entity::find()
        .filter(grants::Column::Id.is_in([
            revoked_recently.clone(),
            young_idle.clone(),
            used_recently.clone(),
            long_idle.clone(),
        ]))
        .all(&h.db)
        .await
        .expect("read grants")
        .into_iter()
        .map(|grant| grant.id)
        .collect();
    for (id, label) in [
        (&revoked_recently, "revoked inside the retention"),
        (&young_idle, "created inside the retention"),
        (&used_recently, "used inside the retention"),
    ] {
        assert!(remaining.contains(id), "a grant {label} stays");
    }
    assert!(
        !remaining.contains(&long_idle),
        "a grant idle past the retention goes"
    );
}

/// Present one code from many tasks at once: at most one exchange wins,
/// and any token it received ends revoked.
pub async fn concurrent_code_replay(h: Arc<Harness>) {
    let user_id = h.create_user().await;
    let bearer = h.sign_in(&user_id).await;
    let client_id = h.register(&[REDIRECT]).await;
    let (verifier, challenge) = pkce();
    let code = h
        .approve(&client_id, REDIRECT, &challenge, "tools:read", &bearer)
        .await;
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let h = h.clone();
        let (client_id, code, verifier) = (client_id.clone(), code.clone(), verifier.clone());
        tasks.push(tokio::spawn(async move {
            let response = h.exchange(&client_id, REDIRECT, &code, &verifier).await;
            (status(&response), body(&response))
        }));
    }
    let mut winners = Vec::new();
    for task in tasks {
        let (code, answer) = task.await.expect("an exchange task");
        if code == 200 {
            winners.push(answer);
        } else {
            assert_eq!(answer["error"], "invalid_grant", "{answer:?}");
        }
    }
    assert!(winners.len() <= 1, "{} exchanges won", winners.len());
    for winner in winners {
        assert_eq!(
            h.verify(winner["access_token"].as_str().expect("access"))
                .await,
            AccessVerification::Invalid(InvalidAccessReason::Revoked),
            "a replayed code revokes what was issued from it"
        );
    }
}
