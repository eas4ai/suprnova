//! Framework-neutral context and driver boundaries exposed to plugins.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::Result;
use crate::abuse::AbuseLimiter;
use crate::auth::FactorGate;
use crate::schema::AuthSchema;
use crate::sessions::{HostSessionApproval, SessionQueries, VerifiedSession, WebSessionBinding};
use crate::storage::{CeremonyStore, TokenStore};

/// Result of running global plugin middleware.
#[derive(Debug)]
pub enum BeforeRequest {
    /// Continue route dispatch without changing the response.
    Continue,
    /// Stop dispatch and return a response.
    Respond(crate::plugin::wire::WireResponse),
    /// Ask the host session query boundary to resolve a bearer credential.
    Bind(BearerCredential),
}

/// Opaque credential supplied to the host session-query boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BearerCredential(String);

impl BearerCredential {
    /// Wrap a carrier-provided bearer value; plugins never receive a session.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    /// Borrow the carrier value for host-owned resolution.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// Storage facade available to plugins. The schema parameter keeps all entity
/// descriptors application-bound while the object remains erased at runtime.
pub trait AuthStorage<S: AuthSchema>: TokenStore + CeremonyStore + Send + Sync {}

impl<S, T> AuthStorage<S> for T
where
    S: AuthSchema,
    T: TokenStore + CeremonyStore + Send + Sync,
{
}

/// At-rest encryption boundary used for plugin-owned opaque data.
#[async_trait]
pub trait Encryptor: Send + Sync {
    /// Encrypt bytes for storage or transport.
    async fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>>;
    /// Decrypt bytes previously returned by [`Encryptor::encrypt`].
    async fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>>;
}

/// A framework-neutral outbound mail message.
#[derive(Clone, Debug, PartialEq)]
pub struct MailMessage {
    /// Stable message name, such as `magic_link` or a plugin namespace.
    pub name: String,
    /// Recipient address or host-defined recipient identifier.
    pub recipient: String,
    /// Structured payload owned by the driver/template layer.
    pub payload: Value,
}

/// Mail-delivery driver implemented by the host.
#[async_trait]
pub trait MailDriver: Send + Sync {
    /// Deliver a typed message without prescribing a mail framework.
    async fn send(&self, message: MailMessage) -> Result<()>;
}

/// Framework-neutral outbound HTTP request.
///
/// `Debug` is hand-implemented rather than derived: Task 4's grant
/// executors render request bodies (PKCE `code_verifier`, `client_secret`,
/// `refresh_token`, signed JWT-bearer assertions) and headers (an X
/// provider's `Authorization: Basic base64(client_id:client_secret)`) into
/// this type before handing it to a host transport, so an incidental
/// `{request:?}` in a log line must never reproduce a live secret.
#[derive(Clone, PartialEq, Eq)]
pub struct HttpRequest {
    /// HTTP method token.
    pub method: String,
    /// Absolute URL.
    pub url: String,
    /// Request headers.
    pub headers: Vec<(String, String)>,
    /// Request body bytes.
    pub body: Vec<u8>,
}

impl fmt::Debug for HttpRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field(
                "headers",
                &self
                    .headers
                    .iter()
                    .map(|(name, _)| (name.as_str(), "[redacted]"))
                    .collect::<Vec<_>>(),
            )
            .field(
                "body",
                &format_args!("[redacted {} bytes]", self.body.len()),
            )
            .finish()
    }
}

/// Framework-neutral outbound HTTP response.
///
/// `Debug` is hand-implemented rather than derived, mirroring
/// [`HttpRequest`]: a token-endpoint response body carries a live access
/// token (and often a refresh token) in plaintext JSON, so an incidental
/// `{response:?}` must never reproduce it.
#[derive(Clone, PartialEq, Eq)]
pub struct HttpResponse {
    /// Status code.
    pub status: u16,
    /// Response headers.
    pub headers: Vec<(String, String)>,
    /// Response body bytes.
    pub body: Vec<u8>,
}

impl fmt::Debug for HttpResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpResponse")
            .field("status", &self.status)
            .field(
                "headers",
                &self
                    .headers
                    .iter()
                    .map(|(name, _)| (name.as_str(), "[redacted]"))
                    .collect::<Vec<_>>(),
            )
            .field(
                "body",
                &format_args!("[redacted {} bytes]", self.body.len()),
            )
            .finish()
    }
}

/// Outbound HTTP transport implemented by the host.
#[async_trait]
pub trait HttpTransport: Send + Sync {
    /// Send a request using host-selected TLS, proxy, and retry policy.
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse>;
}

/// Host route/link resolver used by plugins when constructing absolute links.
#[async_trait]
pub trait LinkGenerator: Send + Sync {
    /// Resolve a named route and parameters to an absolute URL.
    async fn url_for(&self, route_name: &str, params: &[(String, String)]) -> Result<String>;
}

/// Cheap, cloneable handle exposing only approved plugin capabilities.
#[derive(Clone)]
pub struct PluginContext<S: AuthSchema> {
    storage: Arc<dyn AuthStorage<S>>,
    sessions: Arc<dyn SessionQueries>,
    factor_gate: Arc<dyn FactorGate>,
    encryptor: Arc<dyn Encryptor>,
    abuse_limiter: Arc<dyn AbuseLimiter>,
    mail: Arc<dyn MailDriver>,
    http: Arc<dyn HttpTransport>,
    links: Arc<dyn LinkGenerator>,
}

impl<S: AuthSchema> PluginContext<S> {
    /// Construct a context from host-owned driver implementations.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        storage: Arc<dyn AuthStorage<S>>,
        sessions: Arc<dyn SessionQueries>,
        factor_gate: Arc<dyn FactorGate>,
        encryptor: Arc<dyn Encryptor>,
        abuse_limiter: Arc<dyn AbuseLimiter>,
        mail: Arc<dyn MailDriver>,
        http: Arc<dyn HttpTransport>,
        links: Arc<dyn LinkGenerator>,
    ) -> Self {
        Self {
            storage,
            sessions,
            factor_gate,
            encryptor,
            abuse_limiter,
            mail,
            http,
            links,
        }
    }

    /// Access application-bound token and ceremony storage.
    pub fn storage(&self) -> &Arc<dyn AuthStorage<S>> {
        &self.storage
    }
    /// Access query-only session operations.
    pub fn sessions(&self) -> &Arc<dyn SessionQueries> {
        &self.sessions
    }
    /// Resolve a host web binding using the internal host approval witness.
    pub(crate) async fn resolve_web_binding(
        &self,
        binding: &WebSessionBinding,
    ) -> Result<VerifiedSession> {
        self.sessions
            .resolve_web_binding(binding, &HostSessionApproval::authenticated())
            .await
    }
    /// Access the host's factor authorization gate.
    pub fn factor_gate(&self) -> &Arc<dyn FactorGate> {
        &self.factor_gate
    }
    /// Access encrypted-at-rest operations.
    pub fn encryptor(&self) -> &Arc<dyn Encryptor> {
        &self.encryptor
    }
    /// Access abuse limiting.
    pub fn abuse_limiter(&self) -> &Arc<dyn AbuseLimiter> {
        &self.abuse_limiter
    }
    /// Access mail delivery.
    pub fn mail(&self) -> &Arc<dyn MailDriver> {
        &self.mail
    }
    /// Access outbound HTTP transport.
    pub fn http(&self) -> &Arc<dyn HttpTransport> {
        &self.http
    }
    /// Access named-route link generation.
    pub fn links(&self) -> &Arc<dyn LinkGenerator> {
        &self.links
    }
}

/// Initialization-time plugin view.
pub struct InitContext<'a, S: AuthSchema> {
    /// Shared plugin capabilities.
    pub plugin: &'a PluginContext<S>,
}

impl<'a, S: AuthSchema> InitContext<'a, S> {
    /// Construct an initialization view.
    pub fn new(plugin: &'a PluginContext<S>) -> Self {
        Self { plugin }
    }
}

/// Request-time plugin view.
pub struct RequestContext<'a, S: AuthSchema> {
    /// Shared plugin capabilities.
    pub plugin: &'a PluginContext<S>,
    /// Request that selected the plugin route.
    pub request: &'a crate::plugin::wire::WireRequest,
    /// Optional session bound by the host's before-request channel.
    pub session: Option<&'a VerifiedSession>,
}

impl<'a, S: AuthSchema> RequestContext<'a, S> {
    /// Construct a request view without a bound session.
    pub fn new(
        plugin: &'a PluginContext<S>,
        request: &'a crate::plugin::wire::WireRequest,
    ) -> Self {
        Self {
            plugin,
            request,
            session: None,
        }
    }

    /// Construct a request view with an optional host-verified session.
    pub fn with_session(
        plugin: &'a PluginContext<S>,
        request: &'a crate::plugin::wire::WireRequest,
        session: Option<&'a VerifiedSession>,
    ) -> Self {
        Self {
            plugin,
            request,
            session,
        }
    }
}
/// Lifecycle-hook view. It carries capabilities but no request carrier.
pub struct HookContext<'a, S: AuthSchema> {
    /// Shared plugin capabilities.
    pub plugin: &'a PluginContext<S>,
}

impl<'a, S: AuthSchema> HookContext<'a, S> {
    /// Construct a lifecycle-hook view.
    pub fn new(plugin: &'a PluginContext<S>) -> Self {
        Self { plugin }
    }
}
