//! Composition harness for the agent auth suites (AGENT-007 to AGENT-011).
//!
//! Builds the authorization server, agent auth and both plugins over
//! Magnetar's default schema, so the same harness runs on in-memory SQLite
//! and on a live PostgreSQL or MySQL database. The trusted provider's keys
//! are generated here and its key set is served by a scripted pinned
//! client; nothing reaches the network.

#![allow(dead_code)]

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use base64::Engine;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use magnetar::abuse::{AbuseLimiter, AbusePolicy, Permit};
use magnetar::agent_auth::{
    AgentAuth, AgentAuthConfig, AgentAuthServices, AgentAuthStore, AgentEventRecord,
    AgentGrantRecord, IdentityTypes, KeySetFetcher, KeySetPolicy, NewAttempt, ProvisionUsers,
    ProvisioningPolicy, RegistrationRecord, SigningKey, SqlAgentAuthStore, TrustedIssuer,
};
use magnetar::auth::{AuthenticationContext, FactorGate, SignInDecision, VerifiedPrincipal};
use magnetar::authorization_server::{
    AccessVerification, AuthorizationRecord, AuthorizationServer, AuthorizationServerConfig,
    AuthorizationStore, ClientMetadataFetcher, ClientMetadataPolicy, GrantRecord, HostResolver,
    NewTokenRecord, PinnedHttpClient, PinnedResponse, RegisteredClient, ResponseBody,
    RevocationListener, ScopeDefinition, SqlAuthorizationStore, TokenRecord,
};
use magnetar::default_schema::DefaultAuthSchema;
use magnetar::default_schema::sql_stores::SqlSessionStore;
use magnetar::plugin::{
    BearerCredential, DurableLifecycleDelivery, Effect, Encryptor, HttpRequest, HttpResponse,
    HttpTransport, LifecycleEvent, LifecycleEventKind, Method, PluginContext, PluginRegistry,
    PluginResult, WireBody, WireRequest, WireResponse,
};
use magnetar::plugins::agent_auth::AgentAuthPlugin;
use magnetar::plugins::authorization_server::AuthorizationServerPlugin;
use magnetar::sessions::{
    OpaqueConfig, OpaqueSessionProvider, OpaqueSessionStore, SessionGrant, SessionMetadata,
    StoredSession,
};
use magnetar::storage::{NewUser, SeaOrmStorage, UserStore};
use magnetar::{Error, Result};
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::pkcs8::EncodePrivateKey;
use p256::pkcs8::LineEnding;
use parking_lot::Mutex;
use sea_orm::DatabaseConnection;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use url::Url;

#[path = "fakes.rs"]
mod fakes;
#[allow(unused_imports)]
pub use fakes::{RecordingMail, TestLinks};

pub const ISSUER: &str = "https://auth.example.test";
pub const RESOURCE: &str = "https://app.example.test/mcp";
pub const SECOND_RESOURCE: &str = "https://app.example.test/second-mcp";
pub const PROVIDER: &str = "https://provider.example.test";
pub const PROVIDER_HOST: &str = "provider.example.test";
pub const PROVIDER_JWKS: &str = "https://provider.example.test/.well-known/jwks.json";
pub const PROVIDER_NAME: &str = "Acme Agents";
pub const ROGUE: &str = "https://rogue.example.test";
pub const PUBLIC_ADDRESS: &str = "93.184.215.14";
pub const CLIENT_ADDRESS: &str = "198.51.100.7";

/// A fresh P-256 key in PKCS#8 PEM form.
pub fn p256_pem() -> String {
    let secret = p256::SecretKey::random(&mut rand::rngs::OsRng);
    secret
        .to_pkcs8_pem(LineEnding::LF)
        .expect("a PEM key")
        .to_string()
}

/// A service signing key with a fresh P-256 key.
pub fn signing_key(kid: &str) -> (SigningKey, String) {
    let pem = p256_pem();
    (
        SigningKey::from_pem(kid, &pem).expect("a P-256 signing key"),
        pem,
    )
}

/// The authorization server configuration every suite starts from.
pub fn server_config() -> AuthorizationServerConfig {
    let mut config = AuthorizationServerConfig::new(ISSUER, vec![RESOURCE.to_owned()]);
    config.scopes = vec![
        ScopeDefinition::new("tools:read", "Read your tools"),
        ScopeDefinition::new("tools:write", "Change data through tools"),
    ];
    config
}

/// The agent auth configuration every suite starts from: all three types,
/// one trusted provider, `tools:read` before a claim and both scopes after.
pub fn agent_config(key: SigningKey) -> AgentAuthConfig {
    let mut config = AgentAuthConfig::new(key);
    config.service_name = "Example App".to_owned();
    config.identity_types = IdentityTypes {
        anonymous: true,
        service_auth: true,
        identity_assertion: true,
    };
    config.trusted_issuers = vec![TrustedIssuer::new(PROVIDER, PROVIDER_JWKS, PROVIDER_NAME)];
    config.pre_claim_scopes = vec!["tools:read".to_owned()];
    config.post_claim_scopes = vec!["tools:read".to_owned(), "tools:write".to_owned()];
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

struct UnusedFactorGate;

#[async_trait]
impl FactorGate for UnusedFactorGate {
    async fn complete_sign_in(
        &self,
        _principal: VerifiedPrincipal,
        _context: AuthenticationContext,
    ) -> Result<SignInDecision> {
        Err(Error::Internal {
            message: "the agent auth suite signs no one in".to_owned(),
        })
    }

    async fn complete_challenge(&self, _selector: &str, _code: &str) -> Result<SessionGrant> {
        Err(Error::Internal {
            message: "the agent auth suite signs no one in".to_owned(),
        })
    }
}

struct NoHttp;

#[async_trait]
impl HttpTransport for NoHttp {
    async fn send(&self, _request: HttpRequest) -> Result<HttpResponse> {
        Err(Error::Internal {
            message: "agent auth sends nothing through the host transport".to_owned(),
        })
    }
}

/// A resolver that answers from a table.
#[derive(Default)]
pub struct FakeResolver {
    answers: Mutex<HashMap<String, Vec<IpAddr>>>,
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
}

#[async_trait]
impl HostResolver for FakeResolver {
    async fn resolve(&self, host: &str, _port: u16) -> Result<Vec<IpAddr>> {
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

/// A served key set body and its `Cache-Control` header.
type Served = (Vec<u8>, Option<String>);

/// A pinned client serving key sets, counting requests per URL.
#[derive(Default)]
pub struct FakeKeySets {
    documents: Mutex<HashMap<String, Served>>,
    pub requests: Mutex<Vec<(String, SocketAddr)>>,
}

impl FakeKeySets {
    pub fn serve(&self, url: &str, body: Value, cache_control: Option<&str>) {
        self.documents.lock().insert(
            url.to_owned(),
            (
                serde_json::to_vec(&body).expect("a key set"),
                cache_control.map(str::to_owned),
            ),
        );
    }

    pub fn withdraw(&self, url: &str) {
        self.documents.lock().remove(url);
    }

    pub fn request_count(&self, url: &str) -> usize {
        self.requests
            .lock()
            .iter()
            .filter(|(requested, _)| requested == url)
            .count()
    }
}

struct Body(Vec<u8>);

#[async_trait]
impl ResponseBody for Body {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>> {
        if self.0.is_empty() {
            return Ok(None);
        }
        let take = self.0.len().min(4096);
        Ok(Some(self.0.drain(..take).collect()))
    }
}

#[async_trait]
impl PinnedHttpClient for FakeKeySets {
    async fn get(&self, url: &Url, address: SocketAddr) -> Result<PinnedResponse> {
        self.requests.lock().push((url.to_string(), address));
        match self.documents.lock().get(url.as_str()).cloned() {
            Some((body, cache_control)) => Ok(PinnedResponse {
                status: 200,
                cache_control,
                body: Box::new(Body(body)),
            }),
            None => Ok(PinnedResponse {
                status: 404,
                cache_control: None,
                body: Box::new(Body(Vec::new())),
            }),
        }
    }
}

/// An abuse limiter that keeps real fixed windows in memory, so budgets
/// are exercised as configured.
#[derive(Default)]
pub struct WindowLimiter {
    windows: Mutex<HashMap<String, (Instant, u32)>>,
    pub acquired: Mutex<Vec<(String, AbusePolicy)>>,
    pub failing: Mutex<bool>,
}

#[async_trait]
impl AbuseLimiter for WindowLimiter {
    async fn acquire(&self, key: &str, policy: AbusePolicy) -> Result<Permit> {
        self.acquired.lock().push((key.to_owned(), policy));
        if *self.failing.lock() {
            return Err(Error::DependencyUnavailable {
                dependency: "limiter".to_owned(),
                message: "harness outage".to_owned(),
            });
        }
        let mut windows = self.windows.lock();
        let now = Instant::now();
        let entry = windows.entry(key.to_owned()).or_insert((now, 0));
        if now.duration_since(entry.0) >= policy.window {
            *entry = (now, 0);
        }
        if entry.1 >= policy.max_requests {
            return Ok(Permit::Rejected {
                retry_after: policy.window.saturating_sub(now.duration_since(entry.0)),
            });
        }
        entry.1 += 1;
        Ok(Permit::Allowed { retry_after: None })
    }
}

/// The host's durable delivery, recording every audit event it accepts.
#[derive(Default)]
pub struct RecordingEvents {
    pub events: Mutex<Vec<LifecycleEvent>>,
    /// The events the host refused, in order.
    pub refused: Mutex<Vec<LifecycleEvent>>,
    refusals: Mutex<HashMap<LifecycleEventKind, u32>>,
}

impl RecordingEvents {
    pub fn of(&self, kind: LifecycleEventKind) -> Vec<LifecycleEvent> {
        self.events
            .lock()
            .iter()
            .filter(|event| event.kind == kind)
            .cloned()
            .collect()
    }

    /// Refuse the next `times` events of `kind`, as a host whose listener
    /// failed.
    pub fn refuse(&self, kind: LifecycleEventKind, times: u32) {
        self.refusals.lock().insert(kind, times);
    }
}

#[async_trait]
impl DurableLifecycleDelivery for RecordingEvents {
    async fn enqueue(&self, event: LifecycleEvent) -> PluginResult<()> {
        if let Some(left) = self.refusals.lock().get_mut(&event.kind)
            && *left > 0
        {
            *left -= 1;
            self.refused.lock().push(event);
            return Err(magnetar::plugin::PluginError::Request {
                plugin: "test-host".to_owned(),
                message: "the host refused the event".to_owned(),
            });
        }
        self.events.lock().push(event);
        Ok(())
    }
}

/// A pause point: notified when reached, released by the test.
type Gate = (Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>);

/// Failures and lost races to inject into the stores, by method name.
#[derive(Default)]
pub struct Faults {
    failures: Mutex<HashMap<&'static str, u32>>,
    losses: Mutex<HashMap<&'static str, u32>>,
    gates: Mutex<HashMap<&'static str, Gate>>,
    gates_before: Mutex<HashMap<&'static str, Gate>>,
}

impl Faults {
    /// Fail the next `times` calls of `method` with a storage error.
    pub fn fail(&self, method: &'static str, times: u32) {
        self.failures.lock().insert(method, times);
    }

    /// Make the next `times` calls of a conditional `method` lose, as if a
    /// concurrent request won first.
    pub fn lose(&self, method: &'static str, times: u32) {
        self.losses.lock().insert(method, times);
    }

    /// Pause the next call of `method` right after it commits: the first
    /// notify fires when it is reached, and the call waits for the second.
    pub fn gate(&self, method: &'static str) -> Gate {
        let reached = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        self.gates
            .lock()
            .insert(method, (reached.clone(), release.clone()));
        (reached, release)
    }

    /// Pause the next call of `method` before it runs.
    pub fn gate_before(&self, method: &'static str) -> Gate {
        let reached = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        self.gates_before
            .lock()
            .insert(method, (reached.clone(), release.clone()));
        (reached, release)
    }

    async fn pass_before(&self, method: &'static str) {
        let gate = self.gates_before.lock().remove(method);
        if let Some((reached, release)) = gate {
            reached.notify_one();
            release.notified().await;
        }
    }

    async fn pass(&self, method: &'static str) {
        let gate = self.gates.lock().remove(method);
        if let Some((reached, release)) = gate {
            reached.notify_one();
            release.notified().await;
        }
    }

    fn check(&self, method: &'static str) -> Result<()> {
        let mut failures = self.failures.lock();
        match failures.get_mut(method) {
            Some(left) if *left > 0 => {
                *left -= 1;
                Err(Error::Internal {
                    message: format!("injected failure in {method}"),
                })
            }
            _ => Ok(()),
        }
    }

    fn loses(&self, method: &'static str) -> bool {
        let mut losses = self.losses.lock();
        match losses.get_mut(method) {
            Some(left) if *left > 0 => {
                *left -= 1;
                true
            }
            _ => false,
        }
    }
}

/// The agent auth store with injected faults.
pub struct FlakyAgentStore {
    pub inner: SqlAgentAuthStore,
    pub faults: Arc<Faults>,
}

#[async_trait]
impl AgentAuthStore for FlakyAgentStore {
    async fn create_registration(&self, record: RegistrationRecord) -> Result<()> {
        self.faults.check("create_registration")?;
        self.inner.create_registration(record).await
    }
    async fn find_registration(&self, id: &str) -> Result<Option<RegistrationRecord>> {
        self.faults.check("find_registration")?;
        let found = self.inner.find_registration(id).await;
        self.faults.pass("find_registration").await;
        found
    }
    async fn find_by_claim_token(&self, hash: &str) -> Result<Option<RegistrationRecord>> {
        self.faults.check("find_by_claim_token")?;
        self.inner.find_by_claim_token(hash).await
    }
    async fn find_by_attempt_token(&self, hash: &str) -> Result<Option<RegistrationRecord>> {
        self.faults.check("find_by_attempt_token")?;
        self.inner.find_by_attempt_token(hash).await
    }
    async fn find_by_delegation(&self, key: &str) -> Result<Option<RegistrationRecord>> {
        self.faults.check("find_by_delegation")?;
        self.inner.find_by_delegation(key).await
    }
    async fn start_attempt(
        &self,
        id: &str,
        claim_token_hash: &str,
        claim_email: &str,
        attempt: NewAttempt,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool> {
        self.faults.check("start_attempt")?;
        self.inner
            .start_attempt(id, claim_token_hash, claim_email, attempt, now)
            .await
    }
    async fn restart_step_up(
        &self,
        id: &str,
        claim_token_hash: &str,
        claim_expires_at: chrono::DateTime<chrono::Utc>,
        attempt: NewAttempt,
    ) -> Result<bool> {
        self.faults.check("restart_step_up")?;
        self.inner
            .restart_step_up(id, claim_token_hash, claim_expires_at, attempt)
            .await
    }
    async fn record_code_failure(&self, id: &str, hash: &str, max: u32) -> Result<bool> {
        self.faults.check("record_code_failure")?;
        self.inner.record_code_failure(id, hash, max).await
    }
    async fn confirm_claim(
        &self,
        id: &str,
        attempt_token_hash: &str,
        user_code_hash: &str,
        user_id: &str,
        auth_epoch: Option<u64>,
        max_failures: u32,
        now: chrono::DateTime<chrono::Utc>,
        event: &AgentEventRecord,
    ) -> Result<bool> {
        self.faults.check("confirm_claim")?;
        self.faults.pass_before("confirm_claim").await;
        let confirmed = self
            .inner
            .confirm_claim(
                id,
                attempt_token_hash,
                user_code_hash,
                user_id,
                auth_epoch,
                max_failures,
                now,
                event,
            )
            .await;
        self.faults.pass("confirm_claim").await;
        confirmed
    }
    async fn record_poll(
        &self,
        id: &str,
        slot: chrono::DateTime<chrono::Utc>,
        earliest_previous: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool> {
        self.faults.check("record_poll")?;
        self.inner.record_poll(id, slot, earliest_previous).await
    }
    async fn spend_claim_token(&self, id: &str, hash: &str) -> Result<bool> {
        self.faults.check("spend_claim_token")?;
        if self.faults.loses("spend_claim_token") {
            return Ok(false);
        }
        self.inner.spend_claim_token(id, hash).await
    }
    async fn revoke_registration(
        &self,
        id: &str,
        at: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool> {
        self.faults.check("revoke_registration")?;
        self.inner.revoke_registration(id, at).await
    }
    async fn expire_registration(
        &self,
        id: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool> {
        self.faults.check("expire_registration")?;
        if self.faults.loses("expire_registration") {
            return Ok(false);
        }
        self.inner.expire_registration(id, now).await
    }
    async fn finish_cleanup(&self, id: &str, release_delegation: bool) -> Result<()> {
        self.faults.check("finish_cleanup")?;
        self.inner.finish_cleanup(id, release_delegation).await
    }
    async fn expirable_registrations(
        &self,
        now: chrono::DateTime<chrono::Utc>,
        limit: u64,
    ) -> Result<Vec<String>> {
        self.faults.check("expirable_registrations")?;
        self.inner.expirable_registrations(now, limit).await
    }
    async fn live_registrations(&self, limit: u64) -> Result<Vec<String>> {
        self.faults.check("live_registrations")?;
        self.inner.live_registrations(limit).await
    }
    async fn user_registrations(&self, user_id: &str) -> Result<Vec<RegistrationRecord>> {
        self.faults.check("user_registrations")?;
        self.inner.user_registrations(user_id).await
    }
    async fn grant_for(
        &self,
        registration_id: &str,
        resource: &str,
    ) -> Result<Option<AgentGrantRecord>> {
        self.faults.check("grant_for")?;
        self.inner.grant_for(registration_id, resource).await
    }
    async fn grants_of(&self, registration_id: &str) -> Result<Vec<AgentGrantRecord>> {
        self.faults.check("grants_of")?;
        self.inner.grants_of(registration_id).await
    }
    async fn reserve_grant(
        &self,
        record: AgentGrantRecord,
        expected: Option<&str>,
        status: &str,
        assertion_version: u32,
    ) -> Result<bool> {
        self.faults.check("reserve_grant")?;
        self.inner
            .reserve_grant(record, expected, status, assertion_version)
            .await
    }
    async fn guard_mint(
        &self,
        registration_id: &str,
        resource: &str,
        grant_id: &str,
        status: &str,
        assertion_version: u32,
    ) -> Result<bool> {
        self.faults.check("guard_mint")?;
        self.inner
            .guard_mint(
                registration_id,
                resource,
                grant_id,
                status,
                assertion_version,
            )
            .await
    }
    async fn mark_cleanup_pending(&self, id: &str) -> Result<()> {
        self.faults.check("mark_cleanup_pending")?;
        self.inner.mark_cleanup_pending(id).await
    }
    async fn replay_recorded(&self, key: &str, now: chrono::DateTime<chrono::Utc>) -> Result<bool> {
        self.faults.check("replay_recorded")?;
        self.inner.replay_recorded(key, now).await
    }
    async fn record_replay(
        &self,
        key: &str,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool> {
        self.faults.check("record_replay")?;
        self.inner.record_replay(key, expires_at).await
    }
    async fn prune_replays(&self, now: chrono::DateTime<chrono::Utc>) -> Result<u64> {
        self.faults.check("prune_replays")?;
        self.inner.prune_replays(now).await
    }
    async fn undelivered_events(
        &self,
        now: chrono::DateTime<chrono::Utc>,
        limit: u64,
    ) -> Result<Vec<AgentEventRecord>> {
        self.faults.check("undelivered_events")?;
        self.inner.undelivered_events(now, limit).await
    }
    async fn lease_event(
        &self,
        id: &str,
        attempts: u32,
        now: chrono::DateTime<chrono::Utc>,
        until: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool> {
        self.faults.check("lease_event")?;
        self.inner.lease_event(id, attempts, now, until).await
    }
    async fn mark_event_delivered(
        &self,
        id: &str,
        at: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        self.faults.check("mark_event_delivered")?;
        self.inner.mark_event_delivered(id, at).await
    }
    async fn fail_event_attempt(
        &self,
        id: &str,
        error: &str,
        failed_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<()> {
        self.faults.check("fail_event_attempt")?;
        self.inner.fail_event_attempt(id, error, failed_at).await
    }
    async fn retry_failed_events(&self) -> Result<u64> {
        self.faults.check("retry_failed_events")?;
        self.inner.retry_failed_events().await
    }
    async fn failed_event_count(&self) -> Result<u64> {
        self.faults.check("failed_event_count")?;
        self.inner.failed_event_count().await
    }
    async fn prune_events(&self, before: chrono::DateTime<chrono::Utc>) -> Result<u64> {
        self.faults.check("prune_events")?;
        self.inner.prune_events(before).await
    }
}

/// The authorization server store with injected faults.
pub struct FlakyServerStore {
    pub inner: SqlAuthorizationStore,
    pub faults: Arc<Faults>,
}

type At = chrono::DateTime<chrono::Utc>;

#[async_trait]
impl AuthorizationStore for FlakyServerStore {
    async fn insert_client(&self, client: RegisteredClient) -> Result<()> {
        self.inner.insert_client(client).await
    }
    async fn find_client(&self, client_id: &str) -> Result<Option<RegisteredClient>> {
        self.inner.find_client(client_id).await
    }
    async fn create_grant(&self, grant: GrantRecord, tokens: Vec<NewTokenRecord>) -> Result<()> {
        self.faults.check("create_grant")?;
        self.inner.create_grant(grant, tokens).await
    }
    async fn find_token(&self, token_hash: &str) -> Result<Option<TokenRecord>> {
        self.inner.find_token(token_hash).await
    }
    async fn find_grant(&self, grant_id: &str) -> Result<Option<GrantRecord>> {
        self.inner.find_grant(grant_id).await
    }
    async fn rotate_refresh_token(
        &self,
        refresh_token_id: &str,
        grant_id: &str,
        at: At,
        replacements: Vec<NewTokenRecord>,
    ) -> Result<bool> {
        self.inner
            .rotate_refresh_token(refresh_token_id, grant_id, at, replacements)
            .await
    }
    async fn revoke_token(&self, token_id: &str, at: At) -> Result<bool> {
        self.inner.revoke_token(token_id, at).await
    }
    async fn add_tokens(
        &self,
        grant_id: &str,
        tokens: Vec<NewTokenRecord>,
        at: At,
    ) -> Result<bool> {
        self.faults.check("add_tokens")?;
        let added = self.inner.add_tokens(grant_id, tokens, at).await;
        self.faults.pass("add_tokens").await;
        added
    }
    async fn revoke_grant(&self, grant_id: &str, at: At) -> Result<bool> {
        self.faults.check("revoke_grant")?;
        self.inner.revoke_grant(grant_id, at).await
    }
    async fn revoke_user_grants(&self, user_id: &str, at: At) -> Result<u64> {
        self.inner.revoke_user_grants(user_id, at).await
    }
    async fn touch_grant(&self, grant_id: &str, at: At) -> Result<()> {
        self.inner.touch_grant(grant_id, at).await
    }
    async fn list_user_grants(&self, user_id: &str, now: At) -> Result<Vec<GrantRecord>> {
        self.inner.list_user_grants(user_id, now).await
    }
    async fn create_authorization(&self, record: AuthorizationRecord) -> Result<()> {
        self.inner.create_authorization(record).await
    }
    async fn find_authorization(
        &self,
        id: &str,
        kind: &str,
        now: At,
    ) -> Result<Option<AuthorizationRecord>> {
        self.inner.find_authorization(id, kind, now).await
    }
    async fn transition_authorization(
        &self,
        id: &str,
        kind: &str,
        expected: &str,
        next: &str,
        live_at: Option<At>,
    ) -> Result<bool> {
        self.inner
            .transition_authorization(id, kind, expected, next, live_at)
            .await
    }
    async fn consume_authorization(&self, id: &str, kind: &str, now: At) -> Result<bool> {
        self.inner.consume_authorization(id, kind, now).await
    }
    async fn trim_authorizations(
        &self,
        user_id: &str,
        kind: &str,
        keep: u64,
        newest: &str,
    ) -> Result<u64> {
        self.inner
            .trim_authorizations(user_id, kind, keep, newest)
            .await
    }
    async fn prune_expired(&self, now: At, grant_cutoff: At) -> Result<u64> {
        self.inner.prune_expired(now, grant_cutoff).await
    }
}

/// A trusted provider with its own P-256 key.
#[derive(Clone)]
pub struct Provider {
    pub issuer: String,
    pub kid: String,
    pub encoding: EncodingKey,
    pub jwk: Value,
}

impl Provider {
    pub fn new(issuer: &str, kid: &str) -> Self {
        let secret = p256::SecretKey::random(&mut rand::rngs::OsRng);
        let pem = secret.to_pkcs8_pem(LineEnding::LF).expect("a PEM key");
        let point = secret.public_key().to_encoded_point(false);
        let encode = |bytes: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        let jwk = json!({
            "kty": "EC",
            "crv": "P-256",
            "kid": kid,
            "use": "sig",
            "alg": "ES256",
            "x": encode(point.x().expect("x")),
            "y": encode(point.y().expect("y")),
        });
        Self {
            issuer: issuer.to_owned(),
            kid: kid.to_owned(),
            encoding: EncodingKey::from_ec_pem(pem.as_bytes()).expect("an encoding key"),
            jwk,
        }
    }

    pub fn jwks(&self) -> Value {
        json!({"keys": [self.jwk.clone()]})
    }

    /// Sign `claims` as a token of `typ` with this provider's key.
    pub fn sign(&self, typ: &str, claims: &Value) -> String {
        let mut header = Header::new(Algorithm::ES256);
        header.typ = Some(typ.to_owned());
        header.kid = Some(self.kid.clone());
        jsonwebtoken::encode(&header, claims, &self.encoding).expect("a signed token")
    }

    /// The claims of a valid ID-JAG for `subject` with `email` verified.
    pub fn id_jag_claims(&self, subject: &str, email: &str) -> Map<String, Value> {
        let now = chrono::Utc::now().timestamp();
        let claims = json!({
            "iss": self.issuer,
            "sub": subject,
            "aud": ISSUER,
            "client_id": self.issuer,
            "jti": unique("jti"),
            "iat": now,
            "exp": now + 300,
            "auth_time": now - 60,
            "email": email,
            "email_verified": true,
            "name": "Provider-Chosen Name",
            "agent_platform": "test-platform",
            "agent_context_id": "chat-1",
        });
        match claims {
            Value::Object(map) => map,
            _ => Map::new(),
        }
    }

    pub fn id_jag(&self, claims: &Map<String, Value>) -> String {
        self.sign("oauth-id-jag+jwt", &Value::Object(claims.clone()))
    }

    /// A Security Event Token revoking `subject`'s delegation.
    pub fn revocation_event(&self, subject: &str) -> Map<String, Value> {
        let claims = json!({
            "iss": self.issuer,
            "sub": subject,
            "aud": ISSUER,
            "jti": unique("set"),
            "iat": chrono::Utc::now().timestamp(),
            "events": {
                "https://schemas.workos.com/events/agent/auth/identity/assertion/revoked": {}
            },
        });
        match claims {
            Value::Object(map) => map,
            _ => Map::new(),
        }
    }

    pub fn event(&self, claims: &Map<String, Value>) -> String {
        self.sign("secevent+jwt", &Value::Object(claims.clone()))
    }
}

/// The composed world one suite runs in.
pub struct Harness {
    pub db: DatabaseConnection,
    pub storage: Arc<SeaOrmStorage<DefaultAuthSchema>>,
    pub server: Arc<AuthorizationServer>,
    pub engine: Arc<AgentAuth>,
    pub registry: PluginRegistry<DefaultAuthSchema>,
    pub resolver: Arc<FakeResolver>,
    pub key_sets: Arc<FakeKeySets>,
    pub limiter: Arc<WindowLimiter>,
    pub events: Arc<RecordingEvents>,
    pub provider: Provider,
}

/// How to build one harness.
pub struct Options {
    pub config: AgentAuthConfig,
    pub provisioning: Option<Arc<dyn ProvisioningPolicy>>,
    pub provider: Provider,
    /// Faults injected into both stores, when set.
    pub faults: Option<Arc<Faults>>,
    /// The authorization server configuration, when not the default.
    pub server_config: Option<AuthorizationServerConfig>,
    /// One more revocation listener on the authorization server.
    pub revocation_listener: Option<Arc<dyn RevocationListener>>,
    /// The directory agent auth finds users in, when not the Magnetar
    /// store.
    pub users: Option<Arc<dyn magnetar::agent_auth::AgentUserDirectory>>,
}

impl Options {
    pub fn new() -> Self {
        Self {
            config: agent_config(signing_key("service-key-1").0),
            provisioning: None,
            provider: Provider::new(PROVIDER, "provider-key-1"),
            faults: None,
            server_config: None,
            revocation_listener: None,
            users: None,
        }
    }
}

pub async fn sqlite_harness() -> Harness {
    build(magnetar::default_schema::database().await, Options::new()).await
}

pub async fn sqlite_harness_with(options: Options) -> Harness {
    build(magnetar::default_schema::database().await, options).await
}

/// A harness over a live database at `url`, after the default migration.
pub async fn live_database(url: &str) -> DatabaseConnection {
    let db = sea_orm::Database::connect(url)
        .await
        .expect("connect the live backend");
    magnetar::default_schema::migrate(&db)
        .await
        .expect("create the default auth tables and agent auth's");
    db
}

pub async fn build(db: DatabaseConnection, options: Options) -> Harness {
    let storage = Arc::new(SeaOrmStorage::<DefaultAuthSchema>::new(db.clone()));
    let resolver = Arc::new(FakeResolver::default());
    resolver.answer(PROVIDER_HOST, &[PUBLIC_ADDRESS]);
    let key_sets = Arc::new(FakeKeySets::default());
    key_sets.serve(PROVIDER_JWKS, options.provider.jwks(), None);
    let limiter = Arc::new(WindowLimiter::default());
    let events = Arc::new(RecordingEvents::default());
    let fetcher = Arc::new(ClientMetadataFetcher::new(
        resolver.clone(),
        key_sets.clone(),
        ClientMetadataPolicy::default(),
    ));
    let server_store: Arc<dyn AuthorizationStore> = match &options.faults {
        Some(faults) => Arc::new(FlakyServerStore {
            inner: SqlAuthorizationStore::new(db.clone()),
            faults: faults.clone(),
        }),
        None => Arc::new(SqlAuthorizationStore::new(db.clone())),
    };
    let agent_store: Arc<dyn AgentAuthStore> = match &options.faults {
        Some(faults) => Arc::new(FlakyAgentStore {
            inner: SqlAgentAuthStore::new(db.clone()),
            faults: faults.clone(),
        }),
        None => Arc::new(SqlAgentAuthStore::new(db.clone())),
    };
    let server = AuthorizationServer::new(
        options.server_config.clone().unwrap_or_else(server_config),
        server_store,
        storage.clone(),
        limiter.clone(),
        fetcher,
    )
    .expect("a valid authorization server configuration");
    let server = match options.revocation_listener.clone() {
        Some(listener) => server.with_revocation_listener(listener),
        None => server,
    };
    let provisioning = options
        .provisioning
        .unwrap_or_else(|| Arc::new(ProvisionUsers::new(storage.clone())));
    let (server, engine) = AgentAuth::install(
        server,
        options.config,
        AgentAuthServices {
            store: agent_store,
            users: options.users.clone().unwrap_or_else(|| {
                storage.clone() as Arc<dyn magnetar::agent_auth::AgentUserDirectory>
            }),
            provisioning,
            limiter: limiter.clone(),
            key_sets: Arc::new(KeySetFetcher::new(
                resolver.clone(),
                key_sets.clone(),
                KeySetPolicy::default(),
            )),
            events: events.clone(),
        },
    )
    .expect("agent auth installs");
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
        .register(AgentAuthPlugin::new(engine.clone()))
        .build()
        .await
        .expect("both plugins mount");
    Harness {
        db,
        storage,
        server,
        engine,
        registry,
        resolver,
        key_sets,
        limiter,
        events,
        provider: options.provider,
    }
}

impl Harness {
    /// Every outbox event of a registration.
    pub async fn outbox_of(
        &self,
        registration_id: &str,
    ) -> Vec<magnetar::agent_auth::sql::agent_events::Model> {
        use magnetar::agent_auth::sql::agent_events;
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
        agent_events::Entity::find()
            .filter(agent_events::Column::RegistrationId.eq(registration_id.to_owned()))
            .all(&self.db)
            .await
            .expect("read the outbox")
    }
}

pub fn unique(label: &str) -> String {
    format!("{label}-{:016x}", rand::random::<u64>())
}

pub fn sha256_hex(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

pub fn status(response: &WireResponse) -> u16 {
    response.0.status
}

pub fn header(response: &WireResponse, name: &str) -> Option<String> {
    response.0.headers.get(name).cloned()
}

pub fn body(response: &WireResponse) -> Value {
    response.0.body.clone().unwrap_or(Value::Null)
}

pub fn location(response: &WireResponse) -> Option<String> {
    response.effects().iter().find_map(|effect| match effect {
        Effect::Redirect { location, .. } => Some(location.clone()),
        _ => None,
    })
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

/// The claim attempt token inside a `verification_uri`: the sign-in page's
/// `return_to` names the claim page with it.
pub fn attempt_token(verification_uri: &str) -> String {
    let return_to = query_param(verification_uri, "return_to").expect("a return address");
    query_param(&return_to, "claim_attempt_token").expect("a claim attempt token")
}

/// The payload of a JWT, unverified, for checking what a token carries.
pub fn jwt_claims(token: &str) -> Value {
    let payload = token.split('.').nth(1).expect("a JWT payload");
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .expect("base64url");
    serde_json::from_slice(&bytes).expect("a JSON payload")
}

/// The header of a JWT.
pub fn jwt_header(token: &str) -> Value {
    let header = token.split('.').next().expect("a JWT header");
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(header)
        .expect("base64url");
    serde_json::from_slice(&bytes).expect("a JSON header")
}

impl Harness {
    pub async fn send(&self, request: WireRequest, bearer: Option<&str>) -> WireResponse {
        self.registry
            .handle_bound(request, bearer.map(BearerCredential::new))
            .await
            .expect("the plugin answers")
    }

    fn request(method: Method, path: &str, address: Option<&str>) -> WireRequest {
        let mut request = WireRequest::new(method, path);
        if let Some(address) = address {
            request
                .headers
                .insert("x-client-ip".to_owned(), address.to_owned());
        }
        request
    }

    pub async fn post_json_from(
        &self,
        path: &str,
        body: Value,
        address: Option<&str>,
    ) -> WireResponse {
        let mut request = Self::request(Method::Post, path, address);
        request.body = WireBody::Json(body);
        self.send(request, None).await
    }

    pub async fn post_json(&self, path: &str, body: Value) -> WireResponse {
        self.post_json_from(path, body, Some(CLIENT_ADDRESS)).await
    }

    pub async fn post_form(
        &self,
        path: &str,
        fields: &[(&str, &str)],
        bearer: Option<&str>,
    ) -> WireResponse {
        let mut request = Self::request(Method::Post, path, Some(CLIENT_ADDRESS));
        request.body = WireBody::Form(
            fields
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
        );
        self.send(request, bearer).await
    }

    pub async fn post_bytes(&self, path: &str, content_type: &str, bytes: Vec<u8>) -> WireResponse {
        let mut request = Self::request(Method::Post, path, Some(CLIENT_ADDRESS));
        request.body = WireBody::Bytes {
            content_type: Some(content_type.to_owned()),
            bytes,
        };
        self.send(request, None).await
    }

    pub async fn get(
        &self,
        path: &str,
        query: &[(&str, &str)],
        bearer: Option<&str>,
    ) -> WireResponse {
        let mut request = Self::request(Method::Get, path, Some(CLIENT_ADDRESS));
        request.query = query
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        self.send(request, bearer).await
    }

    /// Create a user; `verified` stamps the email as verified.
    pub async fn create_user(&self, email: &str, verified: bool) -> String {
        let user = self
            .storage
            .create_user(NewUser {
                email: email.to_owned(),
                password_hash: Some("fixture-hash".to_owned()),
            })
            .await
            .expect("create a user");
        if verified {
            self.storage
                .mark_email_verified(&user.user_id, chrono::Utc::now())
                .await
                .expect("verify the email");
        }
        user.user_id
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

    pub async fn anonymous(&self) -> Value {
        let response = self
            .post_json("/agent/identity", json!({"type": "anonymous"}))
            .await;
        assert_eq!(status(&response), 200, "{:?}", body(&response));
        body(&response)
    }

    pub async fn service_auth(&self, email: &str) -> Value {
        let response = self
            .post_json(
                "/agent/identity",
                json!({"type": "service_auth", "login_hint": email}),
            )
            .await;
        assert_eq!(status(&response), 200, "{:?}", body(&response));
        body(&response)
    }

    pub async fn present(&self, id_jag: &str) -> WireResponse {
        self.post_json(
            "/agent/identity",
            json!({
                "type": "identity_assertion",
                "assertion_type": "urn:ietf:params:oauth:token-type:id-jag",
                "assertion": id_jag,
            }),
        )
        .await
    }

    pub async fn exchange_with(&self, assertion: &str, extra: &[(&str, &str)]) -> WireResponse {
        let mut fields = vec![
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", assertion),
        ];
        fields.extend_from_slice(extra);
        self.post_form("/oauth/token", &fields, None).await
    }

    pub async fn verify_for(&self, token: &str, resource: &str) -> AccessVerification {
        self.server
            .verify_access_token(token, resource)
            .await
            .expect("verification reads storage")
    }

    pub async fn exchange(&self, assertion: &str) -> WireResponse {
        self.post_form(
            "/oauth/token",
            &[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", assertion),
                ("resource", RESOURCE),
            ],
            None,
        )
        .await
    }

    pub async fn poll(&self, claim_token: &str) -> WireResponse {
        self.post_form(
            "/oauth/token",
            &[
                ("grant_type", "urn:workos:agent-auth:grant-type:claim"),
                ("claim_token", claim_token),
            ],
            None,
        )
        .await
    }

    pub async fn initiate(&self, claim_token: &str, email: &str) -> WireResponse {
        self.post_json(
            "/agent/identity/claim",
            json!({"claim_token": claim_token, "email": email}),
        )
        .await
    }

    pub async fn pending(&self, attempt: &str, bearer: Option<&str>) -> WireResponse {
        self.get(
            "/agent/identity/claim/attempt",
            &[("claim_attempt_token", attempt)],
            bearer,
        )
        .await
    }

    pub async fn confirm(&self, attempt: &str, code: &str, bearer: Option<&str>) -> WireResponse {
        self.post_form(
            "/agent/identity/claim/complete",
            &[("claim_attempt_token", attempt), ("user_code", code)],
            bearer,
        )
        .await
    }

    pub async fn verify(&self, token: &str) -> AccessVerification {
        self.server
            .verify_access_token(token, RESOURCE)
            .await
            .expect("verification reads storage")
    }

    /// Clear every poll timestamp, standing in for the interval passing.
    pub async fn forget_polls(&self) {
        use magnetar::agent_auth::sql::registrations;
        use sea_orm::EntityTrait;
        use sea_orm::sea_query::Expr;
        registrations::Entity::update_many()
            .col_expr(
                registrations::Column::LastPolledAt,
                Expr::value(Option::<chrono::DateTime<chrono::Utc>>::None),
            )
            .exec(&self.db)
            .await
            .expect("clear the poll times");
    }

    /// Move one registration's timestamps, standing in for time passing.
    pub async fn set_registration_time(
        &self,
        registration_id: &str,
        column: magnetar::agent_auth::sql::registrations::Column,
        at: chrono::DateTime<chrono::Utc>,
    ) {
        use magnetar::agent_auth::sql::registrations;
        use sea_orm::sea_query::Expr;
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
        registrations::Entity::update_many()
            .col_expr(column, Expr::value(at))
            .filter(registrations::Column::Id.eq(registration_id.to_owned()))
            .exec(&self.db)
            .await
            .expect("move a registration timestamp");
    }

    /// The authorization server grants the registration's records name,
    /// one per resource.
    pub async fn agent_grant_ids(&self, registration_id: &str) -> Vec<String> {
        SqlAgentAuthStore::new(self.db.clone())
            .grants_of(registration_id)
            .await
            .expect("read the agent grants")
            .into_iter()
            .map(|grant| grant.grant_id)
            .collect()
    }

    pub async fn registration(
        &self,
        registration_id: &str,
    ) -> magnetar::agent_auth::sql::registrations::Model {
        use magnetar::agent_auth::sql::registrations;
        use sea_orm::EntityTrait;
        registrations::Entity::find_by_id(registration_id.to_owned())
            .one(&self.db)
            .await
            .expect("read the registration")
            .expect("the registration exists")
    }

    /// Register anonymously, start a claim for `email` and confirm it as
    /// that user; return the registration body and the confirmed user.
    pub async fn claimed_anonymous(&self, email: &str) -> (Value, String) {
        let registration = self.anonymous().await;
        let user_id = self.create_user(email, true).await;
        let bearer = self.sign_in(&user_id).await;
        let claim_token = registration["claim_token"].as_str().expect("a claim token");
        let started = self.initiate(claim_token, email).await;
        assert_eq!(status(&started), 200, "{:?}", body(&started));
        let started = body(&started);
        let attempt = attempt_token(
            started["claim_attempt"]["verification_uri"]
                .as_str()
                .expect("a verification uri"),
        );
        let code = started["claim_attempt"]["user_code"]
            .as_str()
            .expect("a user code");
        let confirmed = self.confirm(&attempt, code, Some(&bearer)).await;
        assert_eq!(status(&confirmed), 200, "{:?}", body(&confirmed));
        (registration, user_id)
    }
}
