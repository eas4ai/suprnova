//! Shared composition harness for Task 4's grant/revocation/device suites.
//!
//! Adds a real [`FactorGate`] (backed by a fake [`FactorVerifier`], so this
//! harness never needs the `two-factor` feature) and a scripted
//! [`HttpTransport`] on top of `oauth_harness`'s real SeaORM stores, plus a
//! configurable [`MockOAuthProvider`] and [`RecordingRevocationTransport`]
//! for suites that do not need one of the five first-party providers.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::Arc;

use async_trait::async_trait;
use magnetar::auth::{FactorGate, FactorVerifier, OpaqueFactorGate, PreparedFactorProof};
use magnetar::oauth::{
    AuthorizationRequestShape, ClientAuthentication, ClientAuthenticationMaterial,
    EndpointOverrides, InvalidGrantMeaning, OAuthProtocolError, OAuthProvider, OAuthResult,
    ParamPlacement, ProviderIdentity, ProviderResponse, RefreshPolicy, RevocationRequest,
    RevocationTransport, TokenHint, TokenRequestShape,
};
use magnetar::plugin::{HttpRequest, HttpResponse, HttpTransport};
use magnetar::sessions::{OpaqueConfig, OpaqueSessionProvider};
use magnetar::storage::{NewUser, SeaOrmStorage, UserStore};
use magnetar::{Error, Result};
use parking_lot::Mutex;
use secrecy::SecretString;

use super::oauth_harness::{self, OAuthHarness};
use super::storage_schema::StorageSchema;
use super::storage_schema::sql_stores::SqlSessionStore;

/// Configurable second-factor verifier fake: no real TOTP machinery, just a
/// settable enrollment flag and expected code.
#[derive(Default)]
pub struct TestFactorVerifier {
    enrolled: Mutex<bool>,
    code: Mutex<String>,
    claim_count: Mutex<usize>,
}

impl TestFactorVerifier {
    pub fn set_enrolled(&self, enrolled: bool) {
        *self.enrolled.lock() = enrolled;
    }
    pub fn set_code(&self, code: &str) {
        *self.code.lock() = code.to_owned();
    }

    pub fn claim_count(&self) -> usize {
        *self.claim_count.lock()
    }
}

#[async_trait]
impl FactorVerifier for TestFactorVerifier {
    type PreparedProof = bool;

    async fn has_confirmed_enrollment(&self, _user_id: &str) -> Result<bool> {
        Ok(*self.enrolled.lock())
    }

    async fn prepare_code(
        &self,
        _user_id: &str,
        code: &str,
    ) -> Result<PreparedFactorProof<Self::PreparedProof>> {
        if *self.code.lock() == code {
            Ok(PreparedFactorProof::valid(true))
        } else {
            Ok(PreparedFactorProof::invalid(false))
        }
    }

    async fn claim_prepared(&self, _user_id: &str, proof: Self::PreparedProof) -> Result<bool> {
        *self.claim_count.lock() += 1;
        Ok(proof)
    }
}

/// Scripted HTTP transport: a FIFO queue of canned responses/errors,
/// recording every request it was asked to send.
#[derive(Default)]
pub struct ScriptedHttpTransport {
    responses: Mutex<VecDeque<std::result::Result<HttpResponse, Error>>>,
    pub requests: Mutex<Vec<HttpRequest>>,
}

impl ScriptedHttpTransport {
    /// Queue a JSON response with the given status.
    pub fn push_json(&self, status: u16, body: &str) {
        self.responses.lock().push_back(Ok(HttpResponse {
            status,
            headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
            body: body.as_bytes().to_vec(),
        }));
    }
    /// Queue a transport-level failure (network error, not an HTTP status).
    pub fn push_transport_error(&self) {
        self.responses
            .lock()
            .push_back(Err(Error::DependencyUnavailable {
                dependency: "http".to_owned(),
                message: "harness-scripted network failure".to_owned(),
            }));
    }
    pub fn requests(&self) -> Vec<HttpRequest> {
        self.requests.lock().clone()
    }
    pub fn last_request(&self) -> HttpRequest {
        self.requests
            .lock()
            .last()
            .cloned()
            .expect("a request was sent")
    }
    pub fn request_count(&self) -> usize {
        self.requests.lock().len()
    }
}

#[async_trait]
impl HttpTransport for ScriptedHttpTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse> {
        self.requests.lock().push(request);
        self.responses.lock().pop_front().unwrap_or_else(|| {
            Err(Error::Internal {
                message: "ScriptedHttpTransport: no response was scripted for this call".to_owned(),
            })
        })
    }
}

/// Recording revocation transport: records every rendered request and
/// answers with a scripted outcome (defaults to success).
pub struct RecordingRevocationTransport {
    pub requests: Mutex<Vec<Sent>>,
    outcome: Mutex<OAuthResult<()>>,
}

/// A captured [`RevocationRequest`], flattened to owned, inspectable data
/// (the original carries no `Clone`/`Debug`).
#[derive(Clone, Debug)]
pub struct Sent {
    pub method: &'static str,
    pub endpoint: String,
    pub placement: ParamPlacement,
    pub params: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
}

impl Default for RecordingRevocationTransport {
    fn default() -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            outcome: Mutex::new(Ok(())),
        }
    }
}

impl RecordingRevocationTransport {
    pub fn fail(&self, error: OAuthProtocolError) {
        *self.outcome.lock() = Err(error);
    }
    pub fn last(&self) -> Sent {
        self.requests
            .lock()
            .last()
            .cloned()
            .expect("a revocation request was sent")
    }
}

#[async_trait]
impl RevocationTransport for RecordingRevocationTransport {
    async fn send(&self, request: RevocationRequest) -> OAuthResult<()> {
        self.requests.lock().push(Sent {
            method: request.method,
            endpoint: request.endpoint,
            placement: request.placement,
            params: request.params,
            headers: request.headers,
        });
        self.outcome.lock().clone()
    }
}

/// A fully configurable `OAuthProvider` double for grant/revocation suites
/// that do not need one of the five first-party providers' real dossier
/// quirks.
pub struct MockOAuthProvider {
    pub provider_name: &'static str,
    pub client_id_value: String,
    pub client_secret_value: String,
    pub authorization_shape_value: AuthorizationRequestShape,
    pub token_shape_value: TokenRequestShape,
    pub refresh_supported: bool,
    /// `Some` renders a real revocation request through `transport`;
    /// `None` surfaces the "no revocation support" posture as an error.
    /// Read from `endpoints.revocation_endpoint`, mirroring how a real
    /// provider's `revoke()` consults its own override.
    pub endpoints: EndpointOverrides,
    pub transport: Arc<RecordingRevocationTransport>,
}

impl MockOAuthProvider {
    /// Construct a mock whose endpoints default to `{endpoint}`,
    /// `{endpoint}/authorize`, `{endpoint}/userinfo`, and a fixed
    /// `https://mock.test/revoke` -- all driven through the same
    /// [`EndpointOverrides`] a real provider reads, so a test can override
    /// any one of them individually via `provider.endpoints.<field> = ...`
    /// exactly as it would on a real provider's config.
    pub fn new(
        name: &'static str,
        endpoint: &str,
        transport: Arc<RecordingRevocationTransport>,
    ) -> Self {
        Self {
            provider_name: name,
            client_id_value: "mock-client".to_owned(),
            client_secret_value: "mock-secret".to_owned(),
            authorization_shape_value: AuthorizationRequestShape::default(),
            token_shape_value: TokenRequestShape::default(),
            refresh_supported: true,
            endpoints: EndpointOverrides {
                authorization_endpoint: Some(format!("{endpoint}/authorize")),
                token_endpoint: Some(endpoint.to_owned()),
                userinfo_endpoint: Some(format!("{endpoint}/userinfo")),
                revocation_endpoint: Some("https://mock.test/revoke".to_owned()),
                device_authorization_endpoint: None,
                device_token_endpoint: None,
            },
            transport,
        }
    }
}

#[async_trait]
impl OAuthProvider for MockOAuthProvider {
    fn name(&self) -> &'static str {
        self.provider_name
    }

    fn authorization_shape(&self) -> AuthorizationRequestShape {
        self.authorization_shape_value.clone()
    }

    fn token_shape(&self) -> TokenRequestShape {
        self.token_shape_value.clone()
    }

    async fn resolve_identity(&self, _response: ProviderResponse) -> OAuthResult<ProviderIdentity> {
        unimplemented!("not exercised by the grant/revocation/device suites")
    }

    async fn revoke(&self, token: &str, hint: TokenHint) -> OAuthResult<()> {
        let Some(endpoint) = &self.endpoints.revocation_endpoint else {
            return Err(OAuthProtocolError::ProviderConfiguration {
                provider: self.provider_name,
                message: "this provider does not support RFC 7009 revocation".to_owned(),
            });
        };
        let request = RevocationRequest {
            method: "POST",
            endpoint: endpoint.clone(),
            placement: ParamPlacement::Body,
            params: vec![
                ("token".to_owned(), token.to_owned()),
                ("token_type_hint".to_owned(), hint.wire_value().to_owned()),
            ],
            headers: Vec::new(),
        };
        self.transport.send(request).await
    }

    fn refresh_policy(&self) -> RefreshPolicy {
        RefreshPolicy {
            supported: self.refresh_supported,
            token_client_authentication: ClientAuthentication::RequestBody,
            extra_authorization_params: Vec::new(),
            required_scopes: Vec::new(),
            requires_reconsent_for_reissue: false,
            invalid_grant_meaning: InvalidGrantMeaning::OrdinaryRevocation,
        }
    }

    fn client_id(&self) -> &str {
        &self.client_id_value
    }

    async fn client_authentication(&self) -> OAuthResult<ClientAuthenticationMaterial> {
        Ok(ClientAuthenticationMaterial {
            params: vec![("client_secret".to_owned(), self.client_secret_value.clone())],
            headers: Vec::new(),
        })
    }

    fn token_endpoint(&self) -> String {
        self.endpoints
            .token_endpoint
            .clone()
            .expect("mock always sets token_endpoint")
    }

    fn authorization_endpoint(&self) -> String {
        self.endpoints
            .authorization_endpoint
            .clone()
            .expect("mock always sets authorization_endpoint")
    }

    fn userinfo_endpoint(&self) -> Option<String> {
        self.endpoints.userinfo_endpoint.clone()
    }
}

/// The composed world Task 4's grant/revocation/device suites operate in.
pub struct GrantsHarness {
    pub oauth: OAuthHarness,
    pub factors: Arc<TestFactorVerifier>,
    pub gate: Arc<dyn FactorGate>,
    /// Same underlying session store as `gate`; exposed separately so
    /// device-authorization tests can construct a
    /// `DeviceAuthorizationService` (which needs `SessionQueries` for its
    /// best-effort orphan-session cleanup) without duplicating the store.
    pub sessions: Arc<dyn magnetar::sessions::SessionQueries>,
    pub http: Arc<ScriptedHttpTransport>,
}

impl GrantsHarness {
    pub fn storage(&self) -> Arc<SeaOrmStorage<StorageSchema>> {
        self.oauth.storage.clone()
    }
}

pub async fn harness() -> GrantsHarness {
    let oauth = oauth_harness::harness().await;
    let factors = Arc::new(TestFactorVerifier::default());
    let session_store = Arc::new(OpaqueSessionProvider::new(
        Arc::new(SqlSessionStore(oauth.db.clone())),
        OpaqueConfig::default(),
    ));
    let sessions: Arc<dyn magnetar::sessions::SessionQueries> = session_store.clone();
    let gate: Arc<dyn FactorGate> = Arc::new(OpaqueFactorGate::new(
        oauth.storage.clone(),
        factors.clone(),
        oauth.encryptor.clone() as Arc<dyn magnetar::crypto::Encryptor>,
        session_store,
    ));
    GrantsHarness {
        oauth,
        factors,
        gate,
        sessions,
        http: Arc::new(ScriptedHttpTransport::default()),
    }
}

/// Create a test user and return its id.
pub async fn create_user(storage: &Arc<SeaOrmStorage<StorageSchema>>, email: &str) -> String {
    storage
        .create_user(NewUser {
            email: email.to_owned(),
            password_hash: None,
        })
        .await
        .expect("create test user")
        .user_id
}

/// A deterministic HMAC signing key for RFC 7523 JWT-bearer assertion
/// tests -- never a real secret, only used by `jsonwebtoken` to round-trip
/// claims inside the test process.
pub fn test_signing_key() -> SecretString {
    SecretString::from("harness-jwt-bearer-hmac-key-not-a-real-secret".to_owned())
}
