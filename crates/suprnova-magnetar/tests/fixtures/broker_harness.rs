//! Shared composition harness for the token-broker suites (Task 5:
//! `docs/specs/suprnova-magnetar/11-token-broker.md`).
//!
//! Composes a real SeaORM [`ProviderTokenStore`] (reusing `storage_schema`'s
//! fixture entity) with a configurable [`OAuthProvider`] double, a
//! delay-capable scripted transport for deterministic race timing, and a
//! recording reuse hook -- so `tests/token_broker.rs` and
//! `tests/m2m_cache.rs` build under `--features oauth,seaorm-sqlite` alone.
//! `tests/token_broker_concurrency.rs` reuses [`BrokerMockProvider`],
//! [`DelayedScriptedHttpTransport`], and [`RecordingReuseHook`] directly
//! against its own multi-backend database bootstrap.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use magnetar::broker::{BrokerConfig, ReuseHook, TokenBrokerService};
use magnetar::crypto::{AeadEncryptor, CryptoPurpose, Encryptor};
use magnetar::oauth::{
    AuthorizationRequestShape, ClientAuthentication, ClientAuthenticationMaterial,
    InvalidGrantMeaning, OAuthProvider, OAuthProviderRegistry, OAuthResult, ProviderIdentity,
    ProviderResponse, RefreshPolicy, TokenHint, TokenRequestShape,
};
use magnetar::plugin::{HttpRequest, HttpResponse, HttpTransport};
use magnetar::storage::{
    CommitProviderToken, NewProviderToken, ProviderTokenRow, ProviderTokenStore, SeaOrmStorage,
};
use magnetar::{Error, Result};
use parking_lot::Mutex;

use super::storage_schema::{StorageSchema, database};

/// A configurable `OAuthProvider` double for broker suites: unlike
/// `grants_harness::MockOAuthProvider`, its `invalid_grant_meaning` is
/// directly settable per test, exercising spec 11's dossier-driven --
/// never provider-name-branching -- reuse-vs-ordinary-revocation split.
/// Rotation is likewise never a static property of this double: it is
/// entirely driven by whether a scripted response includes a
/// `refresh_token` field, matching `policy::rotated`'s per-response rule.
pub struct BrokerMockProvider {
    pub provider_name: &'static str,
    pub client_id_value: String,
    pub invalid_grant_meaning: InvalidGrantMeaning,
    pub endpoint: String,
}

impl BrokerMockProvider {
    #[must_use]
    pub fn new(name: &'static str, endpoint: &str) -> Self {
        Self {
            provider_name: name,
            client_id_value: format!("{name}-client"),
            invalid_grant_meaning: InvalidGrantMeaning::OrdinaryRevocation,
            endpoint: endpoint.to_owned(),
        }
    }

    #[must_use]
    pub fn with_invalid_grant_meaning(mut self, meaning: InvalidGrantMeaning) -> Self {
        self.invalid_grant_meaning = meaning;
        self
    }
}

#[async_trait]
impl OAuthProvider for BrokerMockProvider {
    fn name(&self) -> &'static str {
        self.provider_name
    }

    fn authorization_shape(&self) -> AuthorizationRequestShape {
        AuthorizationRequestShape::default()
    }

    fn token_shape(&self) -> TokenRequestShape {
        TokenRequestShape::default()
    }

    async fn resolve_identity(&self, _response: ProviderResponse) -> OAuthResult<ProviderIdentity> {
        unimplemented!("not exercised by the broker suites")
    }

    async fn revoke(&self, _token: &str, _hint: TokenHint) -> OAuthResult<()> {
        unimplemented!("not exercised by the broker suites")
    }

    fn client_id(&self) -> &str {
        &self.client_id_value
    }

    fn token_endpoint(&self) -> String {
        self.endpoint.clone()
    }

    fn authorization_endpoint(&self) -> String {
        format!("{}/authorize", self.endpoint)
    }

    fn userinfo_endpoint(&self) -> Option<String> {
        None
    }

    fn refresh_policy(&self) -> RefreshPolicy {
        RefreshPolicy {
            supported: true,
            token_client_authentication: ClientAuthentication::RequestBody,
            extra_authorization_params: Vec::new(),
            required_scopes: Vec::new(),
            requires_reconsent_for_reissue: false,
            invalid_grant_meaning: self.invalid_grant_meaning,
        }
    }

    async fn client_authentication(&self) -> OAuthResult<ClientAuthenticationMaterial> {
        Ok(ClientAuthenticationMaterial {
            params: vec![("client_secret".to_owned(), "broker-mock-secret".to_owned())],
            headers: Vec::new(),
        })
    }
}

/// A queued outcome: an optional delay before the transport returns it,
/// then a response or a transport-level failure.
type Scripted = (Option<Duration>, std::result::Result<HttpResponse, Error>);

/// Delay-capable scripted HTTP transport: a FIFO queue of canned
/// responses/errors, each with an optional `tokio::time::sleep` before it
/// is returned -- the token-broker concurrency suites need this to force
/// deterministic interleaving (a slow "leader" call, observed mid-flight by
/// concurrently racing followers/reclaimers) that `grants_harness`'s
/// zero-delay `ScriptedHttpTransport` cannot produce.
#[derive(Default)]
pub struct DelayedScriptedHttpTransport {
    responses: Mutex<VecDeque<Scripted>>,
    pub requests: Mutex<Vec<HttpRequest>>,
}

impl DelayedScriptedHttpTransport {
    pub fn push_json(&self, status: u16, body: &str) {
        self.push_json_after(Duration::ZERO, status, body);
    }

    pub fn push_json_after(&self, delay: Duration, status: u16, body: &str) {
        self.push_response_after(
            delay,
            HttpResponse {
                status,
                headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
                body: body.as_bytes().to_vec(),
            },
        );
    }

    pub fn push_response_after(&self, delay: Duration, response: HttpResponse) {
        self.responses.lock().push_back((
            if delay.is_zero() { None } else { Some(delay) },
            Ok(response),
        ));
    }

    /// Queue a retriable 5xx response carrying a `Retry-After` header.
    pub fn push_upstream_unavailable(&self, status: u16, retry_after_seconds: u64) {
        self.responses.lock().push_back((
            None,
            Ok(HttpResponse {
                status,
                headers: vec![("Retry-After".to_owned(), retry_after_seconds.to_string())],
                body: Vec::new(),
            }),
        ));
    }

    /// Queue an arbitrary-status response with a plain body and no
    /// headers -- an RFC 6749 §5.2 error body (`invalid_grant`, etc.)
    /// under a non-5xx status.
    pub fn push_status(&self, status: u16, body: &str) {
        self.push_status_after(Duration::ZERO, status, body);
    }

    pub fn push_status_after(&self, delay: Duration, status: u16, body: &str) {
        self.responses.lock().push_back((
            if delay.is_zero() { None } else { Some(delay) },
            Ok(HttpResponse {
                status,
                headers: Vec::new(),
                body: body.as_bytes().to_vec(),
            }),
        ));
    }

    pub fn push_transport_error(&self) {
        self.responses.lock().push_back((
            None,
            Err(Error::DependencyUnavailable {
                dependency: "http".to_owned(),
                message: "harness-scripted network failure".to_owned(),
            }),
        ));
    }

    pub fn requests(&self) -> Vec<HttpRequest> {
        self.requests.lock().clone()
    }

    pub fn request_count(&self) -> usize {
        self.requests.lock().len()
    }
}

#[async_trait]
impl HttpTransport for DelayedScriptedHttpTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse> {
        self.requests.lock().push(request);
        let scripted = self.responses.lock().pop_front();
        let Some((delay, outcome)) = scripted else {
            return Err(Error::Internal {
                message: "DelayedScriptedHttpTransport: no response was scripted for this call"
                    .to_owned(),
            });
        };
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }
        outcome
    }
}

/// Records every fired reuse detection, so tests can assert both "fired
/// exactly once" and "never fired" without inspecting broker internals.
#[derive(Default)]
pub struct RecordingReuseHook {
    calls: Mutex<Vec<(String, String)>>,
}

#[async_trait]
impl ReuseHook for RecordingReuseHook {
    async fn on_reuse_detected(&self, record_id: &str, provider: &str) {
        self.calls
            .lock()
            .push((record_id.to_owned(), provider.to_owned()));
    }
}

impl RecordingReuseHook {
    #[must_use]
    pub fn calls(&self) -> Vec<(String, String)> {
        self.calls.lock().clone()
    }

    #[must_use]
    pub fn count(&self) -> usize {
        self.calls.lock().len()
    }
}

/// Composed world for the single-process broker suites (`token_broker.rs`,
/// `m2m_cache.rs`): a real SeaORM `ProviderTokenStore` over in-memory
/// SQLite plus an `AeadEncryptor`, and a [`Self::seed`] helper that
/// populates a record's initial state through the store's own
/// claim/commit CAS primitives -- never a raw SQL bypass -- so seeded
/// fixtures exercise the identical write path a real provider exchange
/// would.
pub struct BrokerHarness {
    pub db: sea_orm::DatabaseConnection,
    pub store: Arc<dyn ProviderTokenStore>,
    pub encryptor: Arc<AeadEncryptor>,
}

impl BrokerHarness {
    /// Build a [`TokenBrokerService`] over this harness's shared storage
    /// and encryptor, with per-test transport, registry, config, and
    /// reuse hook.
    pub fn service(
        &self,
        transport: Arc<DelayedScriptedHttpTransport>,
        registry: OAuthProviderRegistry,
        config: BrokerConfig,
        reuse_hook: Arc<RecordingReuseHook>,
    ) -> TokenBrokerService {
        TokenBrokerService::new(
            self.store.clone(),
            self.encryptor.clone(),
            transport,
            Arc::new(registry),
            config,
        )
        .with_reuse_hook(reuse_hook)
    }

    /// Provision a fresh record at generation zero and immediately commit
    /// an initial token state onto it through the store's own claim/commit
    /// CAS primitives.
    pub async fn seed(
        &self,
        record_id: &str,
        provider: &str,
        access_token: Option<&str>,
        refresh_token: Option<&str>,
        access_expires_at: Option<DateTime<Utc>>,
    ) -> ProviderTokenRow {
        self.store
            .create_if_missing(NewProviderToken {
                id: record_id.to_owned(),
                provider: provider.to_owned(),
            })
            .await
            .expect("create_if_missing");
        let now = Utc::now();
        let claim_id = "seed-claim";
        let claimed = self
            .store
            .claim(
                record_id,
                0,
                claim_id,
                now + chrono::Duration::seconds(30),
                now,
            )
            .await
            .expect("claim for seed");
        assert!(claimed, "seed claim must succeed on a fresh record");
        let access_ciphertext = access_token
            .map(|value| {
                self.encryptor
                    .encrypt(CryptoPurpose::ProviderToken, value.as_bytes())
                    .unwrap()
            })
            .unwrap_or_default();
        let refresh_ciphertext = refresh_token.map(|value| {
            self.encryptor
                .encrypt(CryptoPurpose::RefreshToken, value.as_bytes())
                .unwrap()
        });
        let raw_payload_ciphertext = self
            .encryptor
            .encrypt(CryptoPurpose::ProviderToken, b"{}")
            .unwrap();
        let committed = self
            .store
            .commit(
                record_id,
                claim_id,
                0,
                CommitProviderToken {
                    access_ciphertext,
                    refresh_ciphertext,
                    raw_payload_ciphertext,
                    token_type: "Bearer".to_owned(),
                    scopes: String::new(),
                    access_expires_at,
                    new_generation: 0,
                },
            )
            .await
            .expect("commit for seed");
        assert!(
            committed,
            "seed commit must succeed under its own fresh claim"
        );
        self.store
            .read(record_id)
            .await
            .expect("read seeded record")
            .expect("seeded record exists")
    }
}

pub async fn harness() -> BrokerHarness {
    let db = database().await;
    let store: Arc<dyn ProviderTokenStore> =
        Arc::new(SeaOrmStorage::<StorageSchema>::new(db.clone()));
    BrokerHarness {
        db,
        store,
        encryptor: Arc::new(AeadEncryptor::new([7; 32])),
    }
}

/// A short-fused [`BrokerConfig`] for tests: real durations but small
/// enough that a `LeaseTimeout`/reclaim path resolves in milliseconds
/// instead of the production defaults' seconds.
#[must_use]
pub fn fast_config(single_flight: bool) -> BrokerConfig {
    BrokerConfig {
        single_flight,
        provider_call_timeout: Duration::from_millis(200),
        lease_grace: Duration::from_millis(50),
        poll_interval: Duration::from_millis(5),
        ..BrokerConfig::default()
    }
}
