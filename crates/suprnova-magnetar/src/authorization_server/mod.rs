//! An OAuth 2.1 authorization server for MCP clients
//! (draft-ietf-oauth-v2-1-13, as the MCP authorization specification of
//! revision 2026-07-28 requires it).
//!
//! A user approves a client to act as them: the authorization code grant
//! with PKCE `S256` only, clients identified by a client metadata document
//! or by dynamic registration (RFC 7591), redirect URIs compared exactly
//! (any port on a loopback address), the `iss` parameter on every
//! authorization response (RFC 9207), access tokens bound to one resource
//! (RFC 8707) and the approved scopes, refresh tokens rotated on every use,
//! revocation (RFC 7009) and authorization server metadata (RFC 8414).
//!
//! [`AuthorizationServer`](crate::authorization_server::AuthorizationServer)
//! is the engine; the
//! [`AuthorizationServerPlugin`](crate::plugins::authorization_server::AuthorizationServerPlugin)
//! mounts it as routes. A resource server checks a token with
//! [`AuthorizationServer::verify_access_token`](crate::authorization_server::AuthorizationServer::verify_access_token),
//! which knows nothing of any host framework.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use chrono::{DateTime, Utc};
use rand::RngCore;
use secrecy::SecretString;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use url::Url;

use crate::abuse::{AbuseLimiter, AbusePolicy};
use crate::storage::UserStore;
use crate::{Error, Result};

pub mod client;
mod flow;
pub mod grants;
pub mod metadata_document;
mod registration;
pub mod sql;
pub mod store;
mod token;

pub use client::{ClientMetadata, ClientSource, ResolvedClient};
pub use flow::{AuthorizeOutcome, AuthorizeRequest, DecisionOutcome, PendingAuthorizationView};
pub use grants::{
    GrantHandler, IssueTokens, IssuedGrant, NewGrant, TokenError, TokenRequest, TokenResponse,
};
pub use metadata_document::{
    ClientMetadataError, ClientMetadataFetcher, ClientMetadataPolicy, HostResolver,
    PinnedHttpClient, PinnedResponse, ReqwestPinnedClient, ResponseBody, SystemResolver,
    is_public_address,
};
pub use registration::{RegistrationError, RegistrationOutcome};
pub use sql::{SqlAuthorizationStore, migrate};
pub use store::{
    ACCESS_TOKEN_KIND, AuthorizationRecord, AuthorizationStore, GrantRecord, NewTokenRecord,
    REFRESH_TOKEN_KIND, RegisteredClient, TokenRecord, USER_GRANT_KIND,
};

/// The prefix of every access token this server mints, so a resource
/// server can tell its tokens from session bearers without a lookup.
pub const ACCESS_TOKEN_PREFIX: &str = "mat_";
/// The prefix of every refresh token this server mints.
pub const REFRESH_TOKEN_PREFIX: &str = "mrt_";

/// The longest token the verification API looks up; anything longer is
/// unknown without touching the database.
const MAX_TOKEN_LENGTH: usize = 512;
/// How often a grant's last-used time is written at most.
const LAST_USED_RESOLUTION: chrono::Duration = chrono::Duration::seconds(60);
/// The largest [`AuthorizationServerConfig::max_pending_per_user`]: the
/// value reaches a query as a signed 64-bit offset, and a user with more
/// than this many approvals in flight is not a user.
const MAX_PENDING_PER_USER: usize = 10_000;
/// How long a spent code is remembered, so a second exchange can revoke
/// what the first one issued.
const CODE_REUSE_WINDOW: Duration = Duration::from_secs(600);

/// One scope the application offers, with the description the approval
/// page shows for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeDefinition {
    /// The scope token, for example `tools:read`.
    pub name: String,
    /// What approving the scope allows, in words for the user.
    pub description: String,
}

impl ScopeDefinition {
    /// Declare a scope.
    #[must_use]
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
        }
    }
}

/// Configuration of one authorization server.
#[derive(Clone, Debug)]
pub struct AuthorizationServerConfig {
    /// The issuer identifier (RFC 8414): an `https` URL without query or
    /// fragment. Endpoints are this URL followed by
    /// [`route_prefix`](Self::route_prefix), and the `iss` parameter
    /// repeats it.
    pub issuer: String,
    /// The path prefix the plugin's routes are mounted under.
    pub route_prefix: String,
    /// The resource indicators (RFC 8707) tokens may be bound to, such as
    /// the MCP endpoint's URL.
    pub resources: Vec<String>,
    /// The scopes the application offers.
    pub scopes: Vec<ScopeDefinition>,
    /// Where a user who is not signed in is sent.
    pub sign_in_url: String,
    /// The query parameter carrying the address to return to after sign-in.
    pub return_to_parameter: String,
    /// The host's approval page. It receives the pending authorization's
    /// identifier in the `authorization` query parameter.
    pub approval_url: String,
    /// How long a pending authorization waits for the user's decision.
    pub pending_lifetime: Duration,
    /// How long an authorization code can be exchanged.
    pub code_lifetime: Duration,
    /// How long an access token works, in whole seconds. Grants issued
    /// without a lifetime of their own follow it as it changes.
    pub access_token_lifetime: Duration,
    /// How long a refresh token works if it is not used, in whole seconds.
    pub refresh_token_lifetime: Duration,
    /// The largest request body (and authorization query) a route reads.
    pub max_request_bytes: usize,
    /// The dynamic registration budget per client address.
    pub registration_policy: AbusePolicy,
    /// The budget per client address for fetching client metadata
    /// documents that are not cached.
    pub metadata_fetch_policy: AbusePolicy,
    /// How many pending authorizations one user keeps, from 1 to 10,000;
    /// creating another drops the oldest. Another site can open the
    /// authorization endpoint in a signed-in user's browser as often as it
    /// likes.
    pub max_pending_per_user: usize,
    /// How long [`AuthorizationServer::prune_expired`] keeps a grant that
    /// was revoked, or that has no token left, before deleting it.
    pub grant_retention: Duration,
}

impl AuthorizationServerConfig {
    /// A configuration for `issuer` serving `resources`, with defaults for
    /// everything else: routes under `/oauth`, sign-in at `/login`, the
    /// approval page at `/oauth/approve`, one-hour access tokens, 30-day
    /// refresh tokens, one-minute codes, a 16 KiB request bound, 20 pending
    /// authorizations per user and 30 days of grant retention.
    #[must_use]
    pub fn new(issuer: impl Into<String>, resources: Vec<String>) -> Self {
        Self {
            issuer: issuer.into(),
            route_prefix: "/oauth".to_owned(),
            resources,
            scopes: Vec::new(),
            sign_in_url: "/login".to_owned(),
            return_to_parameter: "return_to".to_owned(),
            approval_url: "/oauth/approve".to_owned(),
            pending_lifetime: Duration::from_secs(600),
            code_lifetime: Duration::from_secs(60),
            access_token_lifetime: Duration::from_secs(3600),
            refresh_token_lifetime: Duration::from_secs(30 * 24 * 3600),
            max_request_bytes: 16 * 1024,
            registration_policy: AbusePolicy {
                max_requests: 20,
                window: Duration::from_secs(3600),
            },
            metadata_fetch_policy: AbusePolicy {
                max_requests: 30,
                window: Duration::from_secs(60),
            },
            max_pending_per_user: 20,
            grant_retention: Duration::from_secs(30 * 24 * 3600),
        }
    }
}

/// The current authentication epoch of a user, the value a password change
/// or "sign out everywhere" moves. A grant records it at approval and stops
/// working when it moves on, as sessions do.
#[async_trait]
pub trait UserEpochs: Send + Sync {
    /// The user's epoch, or `None` when the user no longer exists.
    async fn current_epoch(&self, user_id: &str) -> Result<Option<u64>>;
}

#[async_trait]
impl<T: UserStore + ?Sized> UserEpochs for T {
    async fn current_epoch(&self, user_id: &str) -> Result<Option<u64>> {
        Ok(self.find_by_id(user_id).await?.map(|user| user.auth_epoch))
    }
}

/// A valid access token's grant, as a resource server needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedAccess {
    /// The grant the token belongs to.
    pub grant_id: String,
    /// The grant family: [`USER_GRANT_KIND`] or an extension's kind.
    pub grant_kind: String,
    /// The token kind: [`ACCESS_TOKEN_KIND`] or an extension's kind.
    pub token_kind: String,
    /// The user the token acts for, if any.
    pub user_id: Option<String>,
    /// The client or agent registration the token was issued to.
    pub client_id: String,
    /// The client's display name.
    pub client_name: String,
    /// The scopes the token carries.
    pub scopes: Vec<String>,
    /// The resource the token is bound to.
    pub resource: String,
    /// When the token expires.
    pub expires_at: DateTime<Utc>,
}

/// Why an access token does not work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidAccessReason {
    /// No such access token.
    Unknown,
    /// The token's lifetime is over.
    Expired,
    /// The token or its grant was revoked.
    Revoked,
    /// The token was issued for another resource.
    WrongResource,
    /// The user's authentication epoch moved on since approval, or the
    /// user no longer exists.
    EpochChanged,
}

/// The outcome of [`AuthorizationServer::verify_access_token`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccessVerification {
    /// The token works. Boxed so the common invalid answer stays small.
    Valid(Box<VerifiedAccess>),
    /// The token does not work, and why.
    Invalid(InvalidAccessReason),
}

/// One client grant a user can see and revoke.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientGrantSummary {
    /// The grant identifier, to revoke it by.
    pub grant_id: String,
    /// The grant family.
    pub kind: String,
    /// The client identifier.
    pub client_id: String,
    /// The client's display name when the user approved it.
    pub client_name: String,
    /// The host shown for the client.
    pub client_host: String,
    /// The approved scopes with their descriptions.
    pub scopes: Vec<ScopeDefinition>,
    /// When the user approved the client.
    pub created_at: DateTime<Utc>,
    /// When the client last used the grant.
    pub last_used_at: Option<DateTime<Utc>>,
}

/// A token the revocation endpoint ended, as a [`RevocationListener`]
/// receives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevokedToken {
    /// The grant the token belongs to.
    pub grant_id: String,
    /// The grant family, so a listener can pick out its own grants.
    pub grant_kind: String,
    /// The client or agent registration the grant was issued to.
    pub client_id: String,
    /// The kind of the token that was presented.
    pub token_kind: String,
    /// The address the revocation request came from, when the host set it.
    pub client_address: Option<String>,
}

/// Told about every token the revocation endpoint ends, so an extension
/// such as agent auth can record it in its audit trail.
#[async_trait]
pub trait RevocationListener: Send + Sync {
    /// One token was revoked. Called after the revocation is stored; a
    /// listener cannot undo or fail it.
    async fn token_revoked(&self, revoked: RevokedToken);
}

/// The authorization server engine.
pub struct AuthorizationServer {
    config: AuthorizationServerConfig,
    issuer: String,
    resources: Vec<String>,
    store: Arc<dyn AuthorizationStore>,
    epochs: Arc<dyn UserEpochs>,
    limiter: Arc<dyn AbuseLimiter>,
    fetcher: Arc<ClientMetadataFetcher>,
    grant_handlers: BTreeMap<String, Arc<dyn GrantHandler>>,
    metadata_fields: Map<String, Value>,
    revocation_listeners: Vec<Arc<dyn RevocationListener>>,
}

/// Metadata members the server writes itself, which an extension may not
/// replace.
const CORE_METADATA_FIELDS: &[&str] = &[
    "issuer",
    "authorization_endpoint",
    "token_endpoint",
    "registration_endpoint",
    "revocation_endpoint",
    "response_types_supported",
    "response_modes_supported",
    "grant_types_supported",
    "code_challenge_methods_supported",
    "token_endpoint_auth_methods_supported",
    "revocation_endpoint_auth_methods_supported",
    "scopes_supported",
    "client_id_metadata_document_supported",
    "authorization_response_iss_parameter_supported",
];

fn config_error(field: &str, message: impl Into<String>) -> Error {
    Error::InvalidInput {
        field: field.to_owned(),
        message: message.into(),
    }
}

/// A scope token (RFC 6749 section 3.3): printable ASCII except space,
/// `"` and `\`.
fn is_scope_token(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte == 0x21 || (0x23..=0x5b).contains(&byte) || (0x5d..=0x7e).contains(&byte)
        })
}

/// A resource indicator in the form the server compares: parsed and
/// serialized, which lowercases the scheme and host. `None` for anything
/// that is not an absolute URI without a fragment (RFC 8707 section 2).
pub(crate) fn normalize_resource(value: &str) -> Option<String> {
    let url = Url::parse(value).ok()?;
    if url.fragment().is_some() || url.cannot_be_a_base() {
        return None;
    }
    Some(url.to_string())
}

fn check_issuer(issuer: &str) -> Result<String> {
    let url = Url::parse(issuer).map_err(|error| config_error("issuer", error.to_string()))?;
    let loopback = matches!(url.host(), Some(url::Host::Domain("localhost")))
        || matches!(url.host(), Some(url::Host::Ipv4(address)) if address.is_loopback())
        || matches!(url.host(), Some(url::Host::Ipv6(address)) if address.is_loopback());
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        return Err(config_error(
            "issuer",
            "must be an https URL (http only on a loopback host)",
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(config_error("issuer", "must not carry a query or fragment"));
    }
    Ok(issuer.trim_end_matches('/').to_owned())
}

impl AuthorizationServer {
    /// Build the server over its stores, after checking the configuration:
    /// a valid issuer, at least one resource, well-formed unique scopes
    /// with descriptions, positive lifetimes and a request bound.
    ///
    /// `store` holds clients, pending authorizations, codes, grants and
    /// tokens; `epochs` reads the user's current authentication epoch;
    /// `limiter` enforces the registration and metadata-fetch budgets.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] naming the configuration field at
    /// fault.
    pub fn new(
        config: AuthorizationServerConfig,
        store: Arc<dyn AuthorizationStore>,
        epochs: Arc<dyn UserEpochs>,
        limiter: Arc<dyn AbuseLimiter>,
        fetcher: Arc<ClientMetadataFetcher>,
    ) -> Result<Self> {
        let issuer = check_issuer(&config.issuer)?;
        if config.resources.is_empty() {
            return Err(config_error("resources", "name at least one resource"));
        }
        let mut resources = Vec::with_capacity(config.resources.len());
        for resource in &config.resources {
            let normalized = normalize_resource(resource).ok_or_else(|| {
                config_error(
                    "resources",
                    format!("{resource:?} is not an absolute URI without a fragment"),
                )
            })?;
            resources.push(normalized);
        }
        let mut seen = std::collections::HashSet::new();
        for scope in &config.scopes {
            if !is_scope_token(&scope.name) {
                return Err(config_error(
                    "scopes",
                    format!("{:?} is not a valid scope token", scope.name),
                ));
            }
            if scope.description.trim().is_empty() {
                return Err(config_error(
                    "scopes",
                    format!("scope {:?} needs a description", scope.name),
                ));
            }
            if !seen.insert(scope.name.clone()) {
                return Err(config_error(
                    "scopes",
                    format!("scope {:?} is declared twice", scope.name),
                ));
            }
        }
        for (field, value) in [
            ("pending_lifetime", config.pending_lifetime),
            ("code_lifetime", config.code_lifetime),
            ("access_token_lifetime", config.access_token_lifetime),
            ("refresh_token_lifetime", config.refresh_token_lifetime),
        ] {
            if value.is_zero() || expiry(Utc::now(), value).is_err() {
                return Err(config_error(
                    field,
                    "must be positive and short enough for a timestamp to hold",
                ));
            }
        }
        // Token lifetimes are stored, and `expires_in` answered, in whole
        // seconds: half a second would be stored as an instant expiry.
        for (field, value) in [
            ("access_token_lifetime", config.access_token_lifetime),
            ("refresh_token_lifetime", config.refresh_token_lifetime),
        ] {
            if !is_whole_seconds(value) {
                return Err(config_error(field, "must be a whole number of seconds"));
            }
        }
        if retention_cutoff(Utc::now(), config.grant_retention).is_err() {
            return Err(config_error(
                "grant_retention",
                "must be short enough for a timestamp to hold",
            ));
        }
        if config.max_pending_per_user == 0 || config.max_pending_per_user > MAX_PENDING_PER_USER {
            return Err(config_error(
                "max_pending_per_user",
                format!("must be between 1 and {MAX_PENDING_PER_USER}"),
            ));
        }
        if config.max_request_bytes == 0 {
            return Err(config_error("max_request_bytes", "must be positive"));
        }
        config.registration_policy.validate()?;
        config.metadata_fetch_policy.validate()?;
        if !config.route_prefix.starts_with('/') {
            return Err(config_error("route_prefix", "must start with /"));
        }
        Ok(Self {
            config,
            issuer,
            resources,
            store,
            epochs,
            limiter,
            fetcher,
            grant_handlers: BTreeMap::new(),
            metadata_fields: Map::new(),
            revocation_listeners: Vec::new(),
        })
    }

    /// Add a grant type the token endpoint dispatches to `handler`. The
    /// metadata document lists it in `grant_types_supported`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Conflict`] for an empty grant type, one of the two
    /// built-in grants, or one already registered.
    pub fn with_grant_handler(mut self, handler: Arc<dyn GrantHandler>) -> Result<Self> {
        let grant_type = handler.grant_type().to_owned();
        if grant_type.trim().is_empty()
            || grant_type == "authorization_code"
            || grant_type == "refresh_token"
            || self.grant_handlers.contains_key(&grant_type)
        {
            return Err(Error::Conflict {
                resource: "grant type".to_owned(),
                message: format!("{grant_type:?} is built in, empty or already registered"),
            });
        }
        self.grant_handlers.insert(grant_type, handler);
        Ok(self)
    }

    /// Add a member to the metadata document, such as agent auth's
    /// `agent_auth` block.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Conflict`] for a member the server writes itself.
    pub fn with_metadata_field(mut self, name: impl Into<String>, value: Value) -> Result<Self> {
        let name = name.into();
        if CORE_METADATA_FIELDS.contains(&name.as_str()) {
            return Err(Error::Conflict {
                resource: "authorization server metadata".to_owned(),
                message: format!("{name:?} is written by the server"),
            });
        }
        self.metadata_fields.insert(name, value);
        Ok(self)
    }

    /// Tell `listener` about every token the revocation endpoint ends.
    #[must_use]
    pub fn with_revocation_listener(mut self, listener: Arc<dyn RevocationListener>) -> Self {
        self.revocation_listeners.push(listener);
        self
    }

    /// The handle an extension issues and revokes tokens through outside a
    /// token request, as agent auth does when a claim or a revocation event
    /// ends a registration's grants.
    #[must_use]
    pub fn token_issuer(&self) -> TokenIssuer<'_> {
        TokenIssuer { server: self }
    }

    /// The configuration the server was built with.
    #[must_use]
    pub fn config(&self) -> &AuthorizationServerConfig {
        &self.config
    }

    /// The issuer identifier, without a trailing slash.
    #[must_use]
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// The path of one of the plugin's routes.
    pub(crate) fn route_path(&self, suffix: &str) -> String {
        format!(
            "{}/{}",
            self.config.route_prefix.trim_end_matches('/'),
            suffix.trim_start_matches('/')
        )
    }

    fn endpoint(&self, suffix: &str) -> String {
        format!("{}{}", self.issuer, self.route_path(suffix))
    }

    /// The authorization server metadata document (RFC 8414). The host
    /// serves it at its well-known locations.
    #[must_use]
    pub fn metadata_document(&self) -> Value {
        let mut grant_types = vec!["authorization_code".to_owned(), "refresh_token".to_owned()];
        grant_types.extend(self.grant_handlers.keys().cloned());
        let scopes: Vec<&str> = self
            .config
            .scopes
            .iter()
            .map(|scope| scope.name.as_str())
            .collect();
        let mut document = self.metadata_fields.clone();
        let core = json!({
            "issuer": self.issuer,
            "authorization_endpoint": self.endpoint("authorize"),
            "token_endpoint": self.endpoint("token"),
            "registration_endpoint": self.endpoint("register"),
            "revocation_endpoint": self.endpoint("revoke"),
            "response_types_supported": ["code"],
            "response_modes_supported": ["query"],
            "grant_types_supported": grant_types,
            "code_challenge_methods_supported": ["S256"],
            "token_endpoint_auth_methods_supported": ["none"],
            "revocation_endpoint_auth_methods_supported": ["none"],
            "scopes_supported": scopes,
            "client_id_metadata_document_supported": true,
            "authorization_response_iss_parameter_supported": true,
        });
        if let Value::Object(core) = core {
            document.extend(core);
        }
        Value::Object(document)
    }

    /// The declared scope names, in declaration order.
    fn declared_scopes(&self) -> Vec<String> {
        self.config
            .scopes
            .iter()
            .map(|scope| scope.name.clone())
            .collect()
    }

    /// Scope names with their descriptions, for the approval page and
    /// listings. A scope the configuration no longer declares keeps its
    /// name as its description.
    fn describe_scopes(&self, names: &[String]) -> Vec<ScopeDefinition> {
        names
            .iter()
            .map(|name| {
                self.config
                    .scopes
                    .iter()
                    .find(|scope| &scope.name == name)
                    .cloned()
                    .unwrap_or_else(|| ScopeDefinition::new(name.clone(), name.clone()))
            })
            .collect()
    }

    /// Parse a space-separated scope parameter into declared scope names,
    /// deduplicated and in declaration order. `Err` names the first scope
    /// the application does not declare.
    fn parse_scope(&self, value: Option<&str>) -> std::result::Result<Vec<String>, String> {
        let requested: Vec<&str> = value.unwrap_or_default().split_whitespace().collect();
        if let Some(unknown) = requested
            .iter()
            .find(|name| !self.config.scopes.iter().any(|scope| scope.name == **name))
        {
            return Err((*unknown).to_owned());
        }
        Ok(self
            .config
            .scopes
            .iter()
            .filter(|scope| requested.contains(&scope.name.as_str()))
            .map(|scope| scope.name.clone())
            .collect())
    }

    /// The configured resource a request names, in its compared form.
    fn accepted_resource(&self, value: &str) -> Option<String> {
        let normalized = normalize_resource(value)?;
        self.resources.contains(&normalized).then_some(normalized)
    }

    /// Check an access token for a resource and return its grant, or why it
    /// does not work. Only an access token works here, of any kind; a
    /// refresh token is unknown.
    ///
    /// # Errors
    ///
    /// Returns an error only when storage fails; every invalid token is an
    /// [`AccessVerification::Invalid`].
    pub async fn verify_access_token(
        &self,
        token: &str,
        resource: &str,
    ) -> Result<AccessVerification> {
        use AccessVerification::Invalid;
        if token.is_empty() || token.len() > MAX_TOKEN_LENGTH {
            return Ok(Invalid(InvalidAccessReason::Unknown));
        }
        let Some(record) = self.store.find_token(&token_digest(token)).await? else {
            return Ok(Invalid(InvalidAccessReason::Unknown));
        };
        if record.kind == REFRESH_TOKEN_KIND {
            return Ok(Invalid(InvalidAccessReason::Unknown));
        }
        if record.revoked_at.is_some() {
            return Ok(Invalid(InvalidAccessReason::Revoked));
        }
        let now = Utc::now();
        if record.expires_at <= now {
            return Ok(Invalid(InvalidAccessReason::Expired));
        }
        let Some(grant) = self.store.find_grant(&record.grant_id).await? else {
            return Ok(Invalid(InvalidAccessReason::Unknown));
        };
        if grant.revoked_at.is_some() {
            return Ok(Invalid(InvalidAccessReason::Revoked));
        }
        if normalize_resource(resource).as_deref() != Some(grant.resource.as_str()) {
            return Ok(Invalid(InvalidAccessReason::WrongResource));
        }
        if !self.epoch_is_current(&grant).await? {
            return Ok(Invalid(InvalidAccessReason::EpochChanged));
        }
        if grant
            .last_used_at
            .is_none_or(|last_used| now - last_used >= LAST_USED_RESOLUTION)
            && let Err(error) = self.store.touch_grant(&grant.id, now).await
        {
            // The token is valid; a failed bookkeeping write must not turn
            // the request away, but it is worth knowing about.
            tracing::warn!(error = %error, grant_id = %grant.id, "recording a grant's last use failed");
        }
        Ok(AccessVerification::Valid(Box::new(VerifiedAccess {
            grant_id: grant.id,
            grant_kind: grant.kind,
            token_kind: record.kind,
            user_id: grant.user_id,
            client_id: grant.client_id,
            client_name: grant.client_name,
            scopes: record.scopes,
            resource: grant.resource,
            expires_at: record.expires_at,
        })))
    }

    /// Whether the grant's user still holds the epoch the grant recorded.
    /// A grant without a user or an epoch has nothing to compare.
    async fn epoch_is_current(&self, grant: &GrantRecord) -> Result<bool> {
        let (Some(user_id), Some(epoch)) = (grant.user_id.as_deref(), grant.auth_epoch) else {
            return Ok(true);
        };
        Ok(self.epochs.current_epoch(user_id).await? == Some(epoch))
    }

    /// The client grants a user has approved and that still work.
    ///
    /// # Errors
    ///
    /// Returns an error when storage fails.
    pub async fn list_grants(&self, user_id: &str) -> Result<Vec<ClientGrantSummary>> {
        let grants = self.store.list_user_grants(user_id, Utc::now()).await?;
        if grants.is_empty() {
            return Ok(Vec::new());
        }
        let current = self.epochs.current_epoch(user_id).await?;
        Ok(grants
            .into_iter()
            .filter(|grant| grant.auth_epoch.is_none() || grant.auth_epoch == current)
            .map(|grant| ClientGrantSummary {
                scopes: self.describe_scopes(&grant.scopes),
                grant_id: grant.id,
                kind: grant.kind,
                client_id: grant.client_id,
                client_name: grant.client_name,
                client_host: grant.client_host,
                created_at: grant.created_at,
                last_used_at: grant.last_used_at,
            })
            .collect())
    }

    /// Revoke one of the user's grants and all of its tokens. Returns
    /// `false` when the grant does not exist, belongs to someone else or is
    /// already revoked, so one user cannot end another's grant.
    ///
    /// # Errors
    ///
    /// Returns an error when storage fails.
    pub async fn revoke_grant(&self, user_id: &str, grant_id: &str) -> Result<bool> {
        let Some(grant) = self.store.find_grant(grant_id).await? else {
            return Ok(false);
        };
        if grant.user_id.as_deref() != Some(user_id) || grant.revoked_at.is_some() {
            return Ok(false);
        }
        self.store.revoke_grant(grant_id, Utc::now()).await
    }

    /// Revoke every grant of a user, for a host that ends a user's sessions
    /// eagerly and wants their clients ended with them. The epoch check
    /// ends them anyway once the epoch moves.
    ///
    /// # Errors
    ///
    /// Returns an error when storage fails.
    pub async fn revoke_all_grants(&self, user_id: &str) -> Result<u64> {
        self.store.revoke_user_grants(user_id, Utc::now()).await
    }

    /// Delete pending authorizations and codes that lapsed and tokens that
    /// expired, then grants that ended more than
    /// [`grant_retention`](AuthorizationServerConfig::grant_retention) ago
    /// (30 days by default): revoked ones, and ones with no token left and
    /// no use since. For a host to run on a schedule. A spent refresh token
    /// is kept until it expires, so its reuse is still caught.
    ///
    /// # Errors
    ///
    /// Returns an error when storage fails.
    pub async fn prune_expired(&self) -> Result<u64> {
        let now = Utc::now();
        let cutoff = retention_cutoff(now, self.config.grant_retention)?;
        self.store.prune_expired(now, cutoff).await
    }

    /// Mint an access token, and a refresh token when asked, for a grant.
    fn mint_tokens(
        &self,
        scopes: &[String],
        access_kind: &str,
        access_lifetime: Duration,
        refresh: bool,
        now: DateTime<Utc>,
    ) -> Result<MintedTokens> {
        let access = new_token(ACCESS_TOKEN_PREFIX);
        let access_expires = expiry(now, access_lifetime)?;
        let mut records = vec![NewTokenRecord {
            id: new_id(),
            kind: access_kind.to_owned(),
            token_hash: token_digest(&access),
            scopes: scopes.to_vec(),
            expires_at: access_expires,
        }];
        let refresh_token = if refresh {
            let token = new_token(REFRESH_TOKEN_PREFIX);
            records.push(NewTokenRecord {
                id: new_id(),
                kind: REFRESH_TOKEN_KIND.to_owned(),
                token_hash: token_digest(&token),
                scopes: scopes.to_vec(),
                expires_at: expiry(now, self.config.refresh_token_lifetime)?,
            });
            Some(SecretString::from(token))
        } else {
            None
        };
        Ok(MintedTokens {
            records,
            response: TokenResponse {
                access_token: SecretString::from(access),
                token_type: "Bearer".to_owned(),
                expires_in: access_lifetime.as_secs(),
                refresh_token,
                scope: scopes.join(" "),
                extra: Map::new(),
            },
        })
    }
}

struct MintedTokens {
    records: Vec<NewTokenRecord>,
    response: TokenResponse,
}

/// The handle a [`GrantHandler`] issues tokens through.
pub struct TokenIssuer<'a> {
    server: &'a AuthorizationServer,
}

impl TokenIssuer<'_> {
    /// Create a grant and mint its tokens, stored and verified exactly as
    /// the built-in grants' tokens are.
    ///
    /// # Errors
    ///
    /// Returns `invalid_request` for an empty kind or client, a refresh
    /// access kind, a zero lifetime or one no timestamp can hold,
    /// `invalid_scope` for an undeclared scope, `invalid_target` for a
    /// resource the server does not accept, and `server_error` when storage
    /// fails.
    pub async fn issue(
        &self,
        grant: NewGrant,
        tokens: IssueTokens,
    ) -> std::result::Result<TokenResponse, TokenError> {
        self.issue_grant(grant, tokens)
            .await
            .map(|issued| issued.response)
    }

    /// [`issue`](Self::issue), answering the new grant's identifier too, so
    /// the extension can revoke the grant when its own state ends it.
    ///
    /// # Errors
    ///
    /// As [`issue`](Self::issue).
    pub async fn issue_grant(
        &self,
        grant: NewGrant,
        tokens: IssueTokens,
    ) -> std::result::Result<IssuedGrant, TokenError> {
        let server = self.server;
        if grant.kind.trim().is_empty() || grant.client_id.trim().is_empty() {
            return Err(TokenError::invalid_request(
                "an issued grant needs a kind and a client",
            ));
        }
        if tokens.access_kind.trim().is_empty() || tokens.access_kind == REFRESH_TOKEN_KIND {
            return Err(TokenError::invalid_request(
                "an access token kind must be named and must not be refresh",
            ));
        }
        let declared = server.declared_scopes();
        if let Some(unknown) = grant.scopes.iter().find(|scope| !declared.contains(scope)) {
            return Err(TokenError::invalid_scope(format!(
                "scope {unknown:?} is not offered"
            )));
        }
        let Some(resource) = server.accepted_resource(&grant.resource) else {
            return Err(TokenError::invalid_target(
                "the resource is not served here",
            ));
        };
        let now = Utc::now();
        if let Some(lifetime) = tokens.access_lifetime
            && (!is_whole_seconds(lifetime) || expiry(now, lifetime).is_err())
        {
            return Err(TokenError::invalid_request(
                "the access token lifetime must be a whole number of seconds, at least one, \
                 and short enough for a timestamp",
            ));
        }
        let access_lifetime = tokens
            .access_lifetime
            .unwrap_or(server.config.access_token_lifetime);
        let minted = server
            .mint_tokens(
                &grant.scopes,
                &tokens.access_kind,
                access_lifetime,
                tokens.refresh,
                now,
            )
            .map_err(token_storage_error)?;
        let grant_id = new_id();
        server
            .store
            .create_grant(
                GrantRecord {
                    id: grant_id.clone(),
                    kind: grant.kind,
                    user_id: grant.user_id,
                    client_id: grant.client_id,
                    client_name: grant.client_name,
                    client_host: grant.client_host,
                    scopes: grant.scopes,
                    resource,
                    auth_epoch: grant.auth_epoch,
                    access_kind: tokens.access_kind,
                    // Only a lifetime the issuer named is kept; otherwise the
                    // grant follows the configuration as it changes.
                    access_lifetime: tokens.access_lifetime,
                    created_at: now,
                    last_used_at: None,
                    revoked_at: None,
                },
                minted.records,
            )
            .await
            .map_err(token_storage_error)?;
        Ok(IssuedGrant {
            grant_id,
            response: minted.response,
        })
    }

    /// Revoke a grant and all of its tokens by its identifier, as an
    /// extension does when its own state ends one (a claimed registration's
    /// pre-claim tokens, for example).
    ///
    /// # Errors
    ///
    /// Returns an error when storage fails.
    pub async fn revoke_grant(&self, grant_id: &str) -> Result<bool> {
        self.server.store.revoke_grant(grant_id, Utc::now()).await
    }
}

/// Log a storage failure behind a token endpoint answer and hide it from
/// the client.
fn token_storage_error(error: Error) -> TokenError {
    tracing::error!(error = %error, "authorization server storage failed");
    TokenError::server_error()
}

/// The moment `lifetime` after `now`. Adding a lifetime with `+` panics
/// when the result outruns what a timestamp holds; this answers an error.
pub(crate) fn expiry(now: DateTime<Utc>, lifetime: Duration) -> Result<DateTime<Utc>> {
    chrono::Duration::from_std(lifetime)
        .ok()
        .and_then(|delta| now.checked_add_signed(delta))
        .ok_or_else(|| Error::InvalidInput {
            field: "lifetime".to_owned(),
            message: "is longer than a timestamp can hold".to_owned(),
        })
}

/// Whether a token lifetime is a whole number of seconds, at least one.
fn is_whole_seconds(lifetime: Duration) -> bool {
    lifetime.as_secs() >= 1 && lifetime.subsec_nanos() == 0
}

/// The moment `retention` before `now`, checked like [`expiry`].
fn retention_cutoff(now: DateTime<Utc>, retention: Duration) -> Result<DateTime<Utc>> {
    chrono::Duration::from_std(retention)
        .ok()
        .and_then(|delta| now.checked_sub_signed(delta))
        .ok_or_else(|| Error::InvalidInput {
            field: "grant_retention".to_owned(),
            message: "is longer than a timestamp can hold".to_owned(),
        })
}

/// Lowercase hex SHA-256 of a token, the only form a token is stored in.
pub(crate) fn token_digest(token: &str) -> String {
    crate::storage::hex_lower(&Sha256::digest(token.as_bytes()))
}

/// 256 random bits from the operating system, as lowercase hex.
fn random_hex() -> String {
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    crate::storage::hex_lower(&bytes)
}

fn new_token(prefix: &str) -> String {
    format!("{prefix}{}", random_hex())
}

fn new_id() -> String {
    let mut bytes = [0_u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    crate::storage::hex_lower(&bytes)
}

/// RFC 7636 S256: `BASE64URL(SHA256(verifier))` without padding.
fn pkce_challenge(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// A code verifier as RFC 7636 section 4.1 defines it: 43 to 128
/// unreserved characters.
fn is_code_verifier(value: &str) -> bool {
    (43..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~'))
}

/// An S256 challenge: exactly 43 base64url characters.
fn is_s256_challenge(value: &str) -> bool {
    value.len() == 43
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
