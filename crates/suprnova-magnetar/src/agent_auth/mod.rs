//! Agent accounts under auth.md v0.6.0.
//!
//! An agent registers at the identity endpoint by one of three types:
//! `anonymous` (no user identity), `service_auth` (the user's email, then a
//! claim ceremony) or `identity_assertion` (an ID-JAG from a provider on
//! the application's trust list). Every path ends in a service-signed
//! identity assertion (`typ: oauth-id-jag+jwt`, `sub` the registration),
//! which the agent exchanges at the authorization server's token endpoint
//! through the JWT bearer grant (RFC 7523) for an access token of the
//! [`AGENT_TOKEN_KIND`](crate::agent_auth::AGENT_TOKEN_KIND) kind and never
//! a refresh token. A claim ceremony, shaped like RFC 8628 device
//! authorization, binds a registration to a signed-in user who types the
//! six-digit code the agent hands them; the agent polls the token endpoint
//! with the claim grant meanwhile.
//!
//! [`AgentAuth::install`](crate::agent_auth::AgentAuth::install) adds both
//! grants and the `agent_auth` metadata block to an
//! [`AuthorizationServer`](crate::authorization_server::AuthorizationServer),
//! and the [`AgentAuthPlugin`](crate::plugins::agent_auth::AgentAuthPlugin)
//! mounts the identity, claim, event and `/auth.md` routes.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use rand::{Rng, RngCore};
use serde_json::{Value, json};

use crate::abuse::{AbuseLimiter, Permit};
use crate::authorization_server::{
    AuthorizationServer, ClientGrantSummary, IssueTokens, NewGrant, ScopeDefinition, TokenError,
    TokenIssuer, TokenResponse,
};
use crate::plugin::{
    DurableLifecycleDelivery, EffectResponse, LifecycleEvent, LifecycleEventKind, WireResponse,
};
use crate::storage::{NewUser, UserStore};
use crate::{Error, Result};

mod claim;
pub mod config;
mod document;
mod events;
mod grants;
mod identity;
mod jwt;
pub mod key_sets;
pub mod sql;
pub mod store;

pub use config::{
    AgentAuthConfig, IdentityTypes, RegistrationLimit, RegistrationLimits, SigningKey,
    TrustedIssuer,
};
pub use key_sets::{KeySetError, KeySetFetcher, KeySetPolicy};
pub use sql::{SqlAgentAuthStore, migrate};
pub use store::{AgentAuthStore, NewAttempt, RegistrationRecord};

use config::RegistrationLimit as Limit;
use store::{STATUS_ACTIVE, STATUS_CLAIMED, STATUS_REVOKED};

/// The grant family of every grant agent auth issues, as
/// [`VerifiedAccess::grant_kind`](crate::authorization_server::VerifiedAccess::grant_kind)
/// reports it.
pub const AGENT_GRANT_KIND: &str = "agent";
/// The kind of every access token agent auth issues, as
/// [`VerifiedAccess::token_kind`](crate::authorization_server::VerifiedAccess::token_kind)
/// reports it, so a resource server tells an agent account from a client
/// acting for a user.
pub const AGENT_TOKEN_KIND: &str = "agent";
/// The JWT bearer grant type (RFC 7523) an agent exchanges its identity
/// assertion through.
pub const JWT_BEARER_GRANT: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";
/// The grant type an agent polls the claim ceremony through.
pub const CLAIM_GRANT: &str = "urn:workos:agent-auth:grant-type:claim";
/// The ID-JAG assertion type.
pub const ID_JAG_ASSERTION_TYPE: &str = "urn:ietf:params:oauth:token-type:id-jag";
/// The Security Event Token schema that revokes a provider's delegation.
pub const ASSERTION_REVOKED_EVENT: &str =
    "https://schemas.workos.com/events/agent/auth/identity/assertion/revoked";

/// A user of the application, as agent auth needs one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentUser {
    /// The application's user identifier.
    pub user_id: String,
    /// The user's email, which a claim is bound to and the second identity
    /// assertion carries.
    pub email: String,
    /// Whether the application verified that email. A claim is bound to an
    /// email, so only a user who proved they hold it can complete one.
    pub email_verified: bool,
    /// The user's authentication epoch, so the user's password change ends
    /// the agent's tokens as it ends sessions.
    pub auth_epoch: Option<u64>,
}

/// How agent auth finds the application's users.
///
/// Every [`UserStore`] is one: it finds users by identifier and by an email
/// the application has verified. An application whose users have verified
/// phone numbers implements this itself to match ID-JAGs by phone.
#[async_trait]
pub trait AgentUserDirectory: Send + Sync {
    /// The user with this identifier.
    async fn find_by_id(&self, user_id: &str) -> Result<Option<AgentUser>>;
    /// The user whose verified email is `email`.
    async fn find_by_verified_email(&self, email: &str) -> Result<Option<AgentUser>>;
    /// The user whose verified phone number is `phone`.
    async fn find_by_verified_phone(&self, phone: &str) -> Result<Option<AgentUser>>;
}

#[async_trait]
impl<T: UserStore + ?Sized> AgentUserDirectory for T {
    async fn find_by_id(&self, user_id: &str) -> Result<Option<AgentUser>> {
        Ok(UserStore::find_by_id(self, user_id)
            .await?
            .map(|user| AgentUser {
                email_verified: user.email_verified_at.is_some(),
                user_id: user.user_id,
                email: user.email,
                auth_epoch: Some(user.auth_epoch),
            }))
    }

    async fn find_by_verified_email(&self, email: &str) -> Result<Option<AgentUser>> {
        // An account whose email the application never verified is not a
        // match: binding a provider identity to it would hand the agent to
        // whoever registered the address first.
        // Looked up in the form Magnetar stores addresses in, as the
        // password plugin does.
        Ok(self
            .find_by_email(&crate::password::normalize_email(email))
            .await?
            .filter(|user| user.email_verified_at.is_some())
            .map(|user| AgentUser {
                user_id: user.user_id,
                email: user.email,
                email_verified: true,
                auth_epoch: Some(user.auth_epoch),
            }))
    }

    async fn find_by_verified_phone(&self, _phone: &str) -> Result<Option<AgentUser>> {
        Ok(None)
    }
}

/// The user signed in to the browser that calls a claim page route, as the
/// host's session says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedInUser {
    /// The user's identifier.
    pub user_id: String,
    /// The authentication epoch the session was issued at, compared with
    /// the user's current one so a session from before a password change
    /// cannot claim.
    pub auth_epoch: Option<u64>,
}

/// The identity an ID-JAG asserts, for the provisioning policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvidedIdentity {
    /// The provider's issuer.
    pub issuer: String,
    /// The provider's subject identifier.
    pub subject: String,
    /// The provider's display name for itself on the trust list.
    pub provider_name: String,
    /// The email the provider verified, if any.
    pub verified_email: Option<String>,
    /// The phone number the provider verified, if any.
    pub verified_phone: Option<String>,
    /// The user's name, as the provider sent it.
    pub name: Option<String>,
}

/// What the provisioning policy decided for an ID-JAG that matches no user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProvisioningDecision {
    /// The policy created this user; the registration is bound to them.
    Provisioned(AgentUser),
    /// The application does not create users this way.
    Refused,
}

/// Decides whether an ID-JAG for an unknown user creates one.
#[async_trait]
pub trait ProvisioningPolicy: Send + Sync {
    /// Create a user for `identity`, or refuse.
    async fn provision(&self, identity: &ProvidedIdentity) -> Result<ProvisioningDecision>;
}

/// The policy for an application that onboards users itself: every unknown
/// identity is refused.
#[derive(Clone, Copy, Debug, Default)]
pub struct RefuseProvisioning;

#[async_trait]
impl ProvisioningPolicy for RefuseProvisioning {
    async fn provision(&self, _identity: &ProvidedIdentity) -> Result<ProvisioningDecision> {
        Ok(ProvisioningDecision::Refused)
    }
}

/// The policy that creates a passwordless user with the provider's
/// verified email, marked verified. An identity with no verified email is
/// refused.
pub struct ProvisionUsers {
    users: Arc<dyn UserStore>,
}

impl ProvisionUsers {
    /// Create users in `users`.
    #[must_use]
    pub fn new(users: Arc<dyn UserStore>) -> Self {
        Self { users }
    }
}

#[async_trait]
impl ProvisioningPolicy for ProvisionUsers {
    async fn provision(&self, identity: &ProvidedIdentity) -> Result<ProvisioningDecision> {
        let Some(email) = identity.verified_email.clone() else {
            return Ok(ProvisioningDecision::Refused);
        };
        match self
            .users
            .create_user(NewUser {
                email,
                password_hash: None,
            })
            .await
        {
            Ok(user) => {
                self.users
                    .mark_email_verified(&user.user_id, Utc::now())
                    .await?;
                Ok(ProvisioningDecision::Provisioned(AgentUser {
                    user_id: user.user_id,
                    email: user.email,
                    email_verified: true,
                    auth_epoch: Some(user.auth_epoch),
                }))
            }
            // Someone created the account meanwhile: it is not this
            // policy's to bind silently, so the presentation is refused and
            // the next one finds the account and steps up.
            Err(Error::Conflict { .. }) => Ok(ProvisioningDecision::Refused),
            Err(error) => Err(error),
        }
    }
}

/// What agent auth runs on besides its configuration.
pub struct AgentAuthServices {
    /// Registrations, ceremonies, grant links and replay records.
    pub store: Arc<dyn AgentAuthStore>,
    /// The application's users.
    pub users: Arc<dyn AgentUserDirectory>,
    /// What an ID-JAG for an unknown user comes to.
    pub provisioning: Arc<dyn ProvisioningPolicy>,
    /// The registration budgets' backend.
    pub limiter: Arc<dyn AbuseLimiter>,
    /// Provider key sets.
    pub key_sets: Arc<KeySetFetcher>,
    /// Where the audit events go. The host persists each and later
    /// delivers it to lifecycle hooks or its own audit log.
    pub events: Arc<dyn DurableLifecycleDelivery>,
}

/// One agent registration bound to a user, for the user's
/// connected-agents page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentRegistrationSummary {
    /// The registration identifier, to revoke it by.
    pub registration_id: String,
    /// How the agent registered: `anonymous`, `service_auth` or
    /// `identity_assertion`.
    pub registration_type: String,
    /// The provider's name from the trust list, for an ID-JAG
    /// registration.
    pub provider: Option<String>,
    /// The scopes the registration holds, with their descriptions.
    pub scopes: Vec<ScopeDefinition>,
    /// When the agent registered.
    pub created_at: DateTime<Utc>,
    /// When the user claimed it, for a claimed registration.
    pub claimed_at: Option<DateTime<Utc>>,
}

/// Everything acting for one user: the clients they approved and the agent
/// registrations bound to them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserConnections {
    /// The authorization server's client grants, without agent grants.
    pub client_grants: Vec<ClientGrantSummary>,
    /// The agent registrations bound to the user.
    pub agent_registrations: Vec<AgentRegistrationSummary>,
}

/// The shared state of the engine, the grant handlers and the revocation
/// listener.
pub(crate) struct Core {
    pub(crate) config: AgentAuthConfig,
    pub(crate) issuer: String,
    pub(crate) origin: String,
    pub(crate) audiences: Vec<String>,
    pub(crate) default_resource: String,
    pub(crate) scope_definitions: Vec<ScopeDefinition>,
    pub(crate) sign_in_url: String,
    pub(crate) return_to_parameter: String,
    pub(crate) token_endpoint: String,
    pub(crate) revocation_endpoint: String,
    pub(crate) store: Arc<dyn AgentAuthStore>,
    pub(crate) users: Arc<dyn AgentUserDirectory>,
    pub(crate) provisioning: Arc<dyn ProvisioningPolicy>,
    pub(crate) limiter: Arc<dyn AbuseLimiter>,
    pub(crate) key_sets: Arc<KeySetFetcher>,
    pub(crate) events: Arc<dyn DurableLifecycleDelivery>,
}

/// The agent auth engine.
pub struct AgentAuth {
    core: Arc<Core>,
    server: Arc<AuthorizationServer>,
}

/// How many registrations one maintenance pass reads at a time.
const BATCH: u64 = 500;

impl AgentAuth {
    /// Add agent auth to `server`: the JWT bearer and claim grants, the
    /// `agent_auth` metadata block and a listener that records revocations
    /// of agent tokens. Returns the server, ready to mount, and the engine.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] for a configuration the server
    /// cannot serve (an undeclared scope, an unaccepted resource, no
    /// identity type, an identity assertion type with no trusted issuer),
    /// and [`Error::Conflict`] when the server already has one of the
    /// grants.
    pub fn install(
        server: AuthorizationServer,
        config: AgentAuthConfig,
        services: AgentAuthServices,
    ) -> Result<(Arc<AuthorizationServer>, Arc<Self>)> {
        let as_config = server.config().clone();
        let declared: Vec<String> = as_config
            .scopes
            .iter()
            .map(|scope| scope.name.clone())
            .collect();
        config.validate(&declared)?;
        let issuer = server.issuer().to_owned();
        let origin = url::Url::parse(&issuer)
            .map(|url| url.origin().ascii_serialization())
            .map_err(|error| Error::InvalidInput {
                field: "issuer".to_owned(),
                message: error.to_string(),
            })?;
        let default_resource = match &config.default_resource {
            Some(resource) => resource.clone(),
            None => as_config
                .resources
                .first()
                .cloned()
                .ok_or_else(|| Error::InvalidInput {
                    field: "default_resource".to_owned(),
                    message: "the authorization server serves no resource".to_owned(),
                })?,
        };
        let audiences = if config.audiences.is_empty() {
            let mut audiences = vec![issuer.clone(), format!("{issuer}/")];
            audiences.extend(as_config.resources.iter().cloned());
            audiences
        } else {
            config.audiences.clone()
        };
        let metadata = server.metadata_document();
        let endpoint = |name: &str| {
            metadata
                .get(name)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_default()
        };
        let core = Arc::new(Core {
            token_endpoint: endpoint("token_endpoint"),
            revocation_endpoint: endpoint("revocation_endpoint"),
            config,
            issuer,
            origin,
            audiences,
            default_resource,
            scope_definitions: as_config.scopes.clone(),
            sign_in_url: as_config.sign_in_url.clone(),
            return_to_parameter: as_config.return_to_parameter.clone(),
            store: services.store,
            users: services.users,
            provisioning: services.provisioning,
            limiter: services.limiter,
            key_sets: services.key_sets,
            events: services.events,
        });
        // A resource the authorization server does not accept would fail
        // every exchange; refusing it here names the mistake at boot.
        if crate::authorization_server::normalize_resource(&core.default_resource).is_none_or(
            |resource| {
                !as_config
                    .resources
                    .iter()
                    .filter_map(|accepted| {
                        crate::authorization_server::normalize_resource(accepted)
                    })
                    .any(|accepted| accepted == resource)
            },
        ) {
            return Err(Error::InvalidInput {
                field: "default_resource".to_owned(),
                message: "the authorization server does not accept this resource".to_owned(),
            });
        }
        let server = server
            .with_grant_handler(Arc::new(grants::JwtBearerGrant::new(core.clone())))?
            .with_grant_handler(Arc::new(grants::ClaimGrant::new(core.clone())))?
            .with_metadata_field("agent_auth", core.metadata_block())?
            .with_revocation_listener(Arc::new(grants::RevocationRecorder::new(core.clone())));
        let server = Arc::new(server);
        let engine = Arc::new(Self {
            core,
            server: server.clone(),
        });
        Ok((server, engine))
    }

    /// The configuration.
    #[must_use]
    pub fn config(&self) -> &AgentAuthConfig {
        &self.core.config
    }

    /// The authorization server the grants are on.
    #[must_use]
    pub fn server(&self) -> &Arc<AuthorizationServer> {
        &self.server
    }

    pub(crate) fn core(&self) -> &Core {
        &self.core
    }

    /// The `agent_auth` member of the authorization server metadata.
    #[must_use]
    pub fn metadata_block(&self) -> Value {
        self.core.metadata_block()
    }

    /// The `/auth.md` document: the instructions an agent reads, written
    /// from the configuration, naming only what the application serves.
    #[must_use]
    pub fn auth_md(&self) -> String {
        self.core.auth_md()
    }

    /// The path of one of the plugin's routes under the route prefix.
    pub(crate) fn route_path(&self, suffix: &str) -> String {
        self.core.route_path(suffix)
    }

    /// Answer the identity endpoint for a parsed JSON body.
    pub async fn identity(&self, body: &Value, client_address: Option<&str>) -> WireResponse {
        self.core.identity(&self.server, body, client_address).await
    }

    /// Answer the claim endpoint: start (or restart) a claim attempt.
    pub async fn initiate_claim(&self, body: &Value, client_address: Option<&str>) -> WireResponse {
        self.core.initiate_claim(body, client_address).await
    }

    /// The pending claim behind `attempt_token`, as JSON for the host's
    /// claim page, for the signed-in user, if any.
    pub async fn pending_claim(
        &self,
        attempt_token: &str,
        signed_in: Option<&SignedInUser>,
    ) -> WireResponse {
        self.core.pending_claim(attempt_token, signed_in).await
    }

    /// Confirm a claim with the user code the signed-in user typed.
    pub async fn confirm_claim(
        &self,
        attempt_token: &str,
        user_code: &str,
        signed_in: Option<&SignedInUser>,
        client_address: Option<&str>,
    ) -> WireResponse {
        self.core
            .confirm_claim(
                &self.server,
                attempt_token,
                user_code,
                signed_in,
                client_address,
            )
            .await
    }

    /// Receive a Security Event Token (RFC 8935 push delivery).
    pub async fn receive_event(
        &self,
        content_type: Option<&str>,
        body: &[u8],
        client_address: Option<&str>,
    ) -> WireResponse {
        self.core
            .receive_event(&self.server, content_type, body, client_address)
            .await
    }

    /// The clients a user approved and the agent registrations bound to
    /// them, for a connected-agents page.
    ///
    /// # Errors
    ///
    /// Returns an error when storage fails.
    pub async fn list_connections(&self, user_id: &str) -> Result<UserConnections> {
        let client_grants = self
            .server
            .list_grants(user_id)
            .await?
            .into_iter()
            .filter(|grant| grant.kind != AGENT_GRANT_KIND)
            .collect();
        let agent_registrations = self
            .core
            .store
            .user_registrations(user_id)
            .await?
            .into_iter()
            .map(|record| AgentRegistrationSummary {
                provider: record
                    .issuer
                    .as_deref()
                    .and_then(|issuer| self.core.config.trusted(issuer))
                    .map(|entry| entry.display_name.clone()),
                scopes: self.core.describe(&record.scopes),
                registration_id: record.id,
                registration_type: record.kind,
                created_at: record.created_at,
                claimed_at: record.claimed_at,
            })
            .collect();
        Ok(UserConnections {
            client_grants,
            agent_registrations,
        })
    }

    /// Revoke one agent registration bound to `user_id`, and every token
    /// issued from it. Returns `false` when the registration does not
    /// exist, is someone else's or is already revoked.
    ///
    /// # Errors
    ///
    /// Returns an error when storage fails.
    pub async fn revoke_registration(&self, user_id: &str, registration_id: &str) -> Result<bool> {
        let Some(record) = self.core.store.find_registration(registration_id).await? else {
            return Ok(false);
        };
        if record.user_id.as_deref() != Some(user_id) || record.status == STATUS_REVOKED {
            return Ok(false);
        }
        self.core
            .end_registration(&self.server.token_issuer(), &record, None, &[])
            .await
    }

    /// Revoke every agent registration of the application and every token
    /// issued from them, for incident response. Returns how many
    /// registrations were revoked.
    ///
    /// # Errors
    ///
    /// Returns an error when storage fails. What was revoked before the
    /// failure stays revoked; a registration whose own row was revoked but
    /// whose grants were not is named in the log, and its tokens end with
    /// their lifetime since no assertion of it exchanges any more.
    pub async fn revoke_all_agent_credentials(&self) -> Result<u64> {
        let mut revoked = 0;
        loop {
            let ids = self.core.store.live_registrations(BATCH).await?;
            if ids.is_empty() {
                return Ok(revoked);
            }
            let before = revoked;
            for id in ids {
                if let Some(record) = self.core.store.find_registration(&id).await?
                    && self
                        .core
                        .end_registration(&self.server.token_issuer(), &record, None, &[])
                        .await?
                {
                    revoked += 1;
                }
            }
            if revoked == before {
                // Nothing in the batch could be revoked; stop rather than
                // read the same rows again.
                return Ok(revoked);
            }
        }
    }

    /// Mark every unclaimed or pending registration whose window passed as
    /// expired, end its tokens and record `registration.expired`. For a
    /// host to run on a schedule. Returns how many expired.
    ///
    /// # Errors
    ///
    /// Returns an error when storage fails.
    pub async fn expire_registrations(&self) -> Result<u64> {
        let mut expired = 0;
        loop {
            let now = Utc::now();
            let ids = self.core.store.expirable_registrations(now, BATCH).await?;
            if ids.is_empty() {
                return Ok(expired);
            }
            for id in ids {
                if !self.core.store.expire_registration(&id, now).await? {
                    continue;
                }
                expired += 1;
                let record = self.core.store.find_registration(&id).await?;
                self.core
                    .revoke_grants(&self.server.token_issuer(), &id)
                    .await?;
                // No request causes an expiry; the event carries the address
                // the registration came from.
                let (kind, address) = record
                    .map(|record| (record.kind, record.client_address))
                    .unwrap_or_default();
                self.core
                    .emit(
                        LifecycleEventKind::AgentRegistrationExpired,
                        None,
                        &[("registration_id", id.clone()), ("registration_type", kind)],
                        address.as_deref(),
                    )
                    .await;
            }
        }
    }

    /// Delete replay records of provider tokens that can no longer be
    /// replayed. For a host to run on a schedule. Returns how many went.
    ///
    /// # Errors
    ///
    /// Returns an error when storage fails.
    pub async fn prune_replay_records(&self) -> Result<u64> {
        self.core.store.prune_replays(Utc::now()).await
    }
}

impl Core {
    pub(crate) fn route_path(&self, suffix: &str) -> String {
        format!(
            "{}/{}",
            self.config.route_prefix.trim_end_matches('/'),
            suffix.trim_start_matches('/')
        )
    }

    pub(crate) fn endpoint(&self, suffix: &str) -> String {
        format!("{}{}", self.issuer, self.route_path(suffix))
    }

    pub(crate) fn skill_url(&self) -> String {
        format!("{}{}", self.issuer, self.config.skill_path)
    }

    /// The scopes with their descriptions; a scope the configuration no
    /// longer declares keeps its name as its description.
    pub(crate) fn describe(&self, names: &[String]) -> Vec<ScopeDefinition> {
        names
            .iter()
            .map(|name| {
                self.scope_definitions
                    .iter()
                    .find(|scope| &scope.name == name)
                    .cloned()
                    .unwrap_or_else(|| ScopeDefinition::new(name.clone(), name.clone()))
            })
            .collect()
    }

    /// The address the user opens: the application's sign-in page, which
    /// returns to the claim page with the attempt token.
    pub(crate) fn verification_uri(&self, attempt_token: &str) -> String {
        let claim_page = append_parameter(
            &self.config.claim_page_url,
            "claim_attempt_token",
            attempt_token,
        );
        let sign_in = if self.sign_in_url.starts_with("https://")
            || self.sign_in_url.starts_with("http://")
        {
            self.sign_in_url.clone()
        } else {
            format!("{}{}", self.origin, self.sign_in_url)
        };
        append_parameter(&sign_in, &self.return_to_parameter, &claim_page)
    }

    /// Consume one registration from both budgets of `limit`: the client
    /// address first, then the application. `Ok(Some(_))` is a refusal
    /// with the time to wait. Without an address only the application's
    /// budget applies, as auth.md asks.
    pub(crate) async fn admit(
        &self,
        limit: Limit,
        kind: &str,
        client_address: Option<&str>,
    ) -> Result<Option<Duration>> {
        if let Some(address) = client_address {
            let key = crate::plugins::abuse_key(&format!("agent-auth.{kind}.address"), address);
            if let Permit::Rejected { retry_after } =
                self.limiter.acquire(&key, limit.per_address).await?
            {
                return Ok(Some(retry_after));
            }
        }
        let key =
            crate::plugins::abuse_key(&format!("agent-auth.{kind}.application"), "application");
        match self.limiter.acquire(&key, limit.per_application).await? {
            Permit::Rejected { retry_after } => Ok(Some(retry_after)),
            Permit::Allowed { .. } => Ok(None),
        }
    }

    /// Record one audit event. The mutation it describes is committed, so
    /// a failure to enqueue is logged, never answered.
    pub(crate) async fn emit(
        &self,
        kind: LifecycleEventKind,
        user_id: Option<&str>,
        attributes: &[(&str, String)],
        client_address: Option<&str>,
    ) {
        let mut event = LifecycleEvent::new(
            format!("agent-auth:{}", random_hex(16)),
            kind,
            user_id.unwrap_or_default(),
        );
        for (name, value) in attributes {
            event = event.with_attribute(*name, value.clone());
        }
        if let Some(address) = client_address {
            event = event.with_attribute("client_address", address);
        }
        if let Err(error) = self.events.enqueue(event).await {
            tracing::error!(error = %error, ?kind, "recording an agent auth audit event failed");
        }
    }

    /// Revoke the authorization server grant a registration names, trying
    /// twice since a failed write is usually a transient lock or pool
    /// timeout. Returns how many were live: zero or one.
    pub(crate) async fn revoke_grants(
        &self,
        issuer: &TokenIssuer<'_>,
        registration_id: &str,
    ) -> Result<u64> {
        let Some(record) = self.store.find_registration(registration_id).await? else {
            return Ok(0);
        };
        let Some(grant_id) = record.grant_id else {
            return Ok(0);
        };
        let mut outcome = issuer.revoke_grant(&grant_id).await;
        if outcome.is_err() {
            outcome = issuer.revoke_grant(&grant_id).await;
        }
        match outcome {
            Ok(revoked) => Ok(u64::from(revoked)),
            Err(error) => {
                tracing::error!(
                    error = %error,
                    grant_id,
                    registration_id,
                    "revoking an agent grant failed"
                );
                Err(error)
            }
        }
    }

    /// Revoke a registration and every token issued from it, then record
    /// `registration.revoked`. The registration row is written before any
    /// grant, the order every path takes, and a grant issued meanwhile is
    /// caught by the issuing path's own recheck.
    pub(crate) async fn end_registration(
        &self,
        issuer: &TokenIssuer<'_>,
        record: &RegistrationRecord,
        client_address: Option<&str>,
        extra: &[(&str, String)],
    ) -> Result<bool> {
        if !self
            .store
            .revoke_registration(&record.id, Utc::now())
            .await?
        {
            return Ok(false);
        }
        self.revoke_grants(issuer, &record.id).await?;
        let mut attributes = vec![
            ("registration_id", record.id.clone()),
            ("registration_type", record.kind.clone()),
        ];
        if let Some(issuer) = &record.issuer {
            attributes.push(("iss", issuer.clone()));
        }
        if let Some(subject) = &record.subject {
            attributes.push(("sub", subject.clone()));
        }
        attributes.extend(extra.iter().cloned());
        self.emit(
            LifecycleEventKind::AgentRegistrationRevoked,
            record.user_id.as_deref(),
            &attributes,
            client_address,
        )
        .await;
        Ok(true)
    }

    /// Mint an access token for a registration under its one grant. The
    /// registration's grant is reused when it still fits (live, the same
    /// user, scopes and resource); otherwise it is revoked and replaced, so
    /// a registration never holds more than one live grant however often
    /// it exchanges.
    ///
    /// A replacement grant's identifier is written to the registration
    /// before the grant exists, so no live grant is ever left without the
    /// record that names it, and the registration is read once more after
    /// the grant is issued: a claim or a revocation that landed meanwhile
    /// revokes the new grant instead of letting its token out.
    pub(crate) async fn issue_for_registration(
        &self,
        issuer: &TokenIssuer<'_>,
        record: &RegistrationRecord,
        user: Option<&AgentUser>,
        scopes: Vec<String>,
        resource: String,
    ) -> std::result::Result<TokenResponse, TokenError> {
        let user_id = user.map(|user| user.user_id.as_str());
        if let Some(grant_id) = &record.grant_id
            && let Some(response) = issuer
                .issue_on_grant(grant_id, user_id, &scopes, &resource)
                .await?
        {
            return Ok(response);
        }
        // The grant no longer fits: it goes before its replacement is
        // recorded, so it is never live without a record.
        if let Some(old) = &record.grant_id {
            issuer.revoke_grant(old).await.map_err(storage_failure)?;
        }
        let new_grant = random_hex(16);
        if !self
            .store
            .reserve_grant(
                &record.id,
                record.grant_id.as_deref(),
                &new_grant,
                &record.status,
                record.assertion_version,
            )
            .await
            .map_err(storage_failure)?
        {
            // Another exchange replaced the grant first, or the registration
            // changed: use the winner's grant if the registration still
            // stands as it was read.
            let current = self
                .store
                .find_registration(&record.id)
                .await
                .map_err(storage_failure)?;
            if let Some(current) = current
                && current.status == record.status
                && current.assertion_version == record.assertion_version
                && current.user_id == record.user_id
                && let Some(grant_id) = &current.grant_id
                && let Some(response) = issuer
                    .issue_on_grant(grant_id, user_id, &scopes, &resource)
                    .await?
            {
                return Ok(response);
            }
            return Err(TokenError::invalid_grant(
                "the registration changed while the token was issued",
            ));
        }
        let issued = issuer
            .issue_grant_as(
                new_grant.clone(),
                NewGrant {
                    kind: AGENT_GRANT_KIND.to_owned(),
                    user_id: user_id.map(str::to_owned),
                    client_id: record.id.clone(),
                    client_name: self.client_name(record),
                    client_host: self.client_host(record),
                    scopes,
                    resource,
                    auth_epoch: user.and_then(|user| user.auth_epoch),
                },
                IssueTokens {
                    access_kind: AGENT_TOKEN_KIND.to_owned(),
                    refresh: false,
                    access_lifetime: None,
                },
            )
            .await?;
        let still_valid = match self.store.find_registration(&record.id).await {
            Ok(current) => current.is_some_and(|current| {
                current.status == record.status
                    && current.assertion_version == record.assertion_version
                    && current.user_id == record.user_id
                    && current.grant_id.as_deref() == Some(new_grant.as_str())
            }),
            Err(error) => {
                tracing::error!(error = %error, "rereading an agent registration failed");
                false
            }
        };
        if !still_valid {
            self.revoke_issued(issuer, &new_grant).await;
            return Err(TokenError::invalid_grant(
                "the registration changed while the token was issued",
            ));
        }
        Ok(issued.response)
    }

    async fn revoke_issued(&self, issuer: &TokenIssuer<'_>, grant_id: &str) {
        for attempt in 1..=2 {
            match issuer.revoke_grant(grant_id).await {
                Ok(_) => return,
                Err(error) => {
                    tracing::error!(error = %error, grant_id, attempt, "revoking an agent grant failed");
                }
            }
        }
    }

    fn client_name(&self, record: &RegistrationRecord) -> String {
        match record
            .issuer
            .as_deref()
            .and_then(|issuer| self.config.trusted(issuer))
        {
            Some(entry) => format!("Agent through {}", entry.display_name),
            None => format!("Agent ({})", record.kind),
        }
    }

    fn client_host(&self, record: &RegistrationRecord) -> String {
        record
            .issuer
            .as_deref()
            .and_then(|issuer| url::Url::parse(issuer).ok())
            .and_then(|url| url.host_str().map(str::to_owned))
            .unwrap_or_default()
    }

    /// Whether the user's authentication epoch moved on since the
    /// registration was bound: a password change or signing out everywhere
    /// ends a bound agent as it ends sessions. A user with no epoch has
    /// nothing to compare.
    pub(crate) fn epoch_moved(record: &RegistrationRecord, user: &AgentUser) -> bool {
        user.auth_epoch.is_some() && user.auth_epoch != record.auth_epoch
    }

    /// Whether a registration is bound to a user and holds its full scopes.
    pub(crate) fn is_bound(record: &RegistrationRecord) -> bool {
        (record.status == STATUS_ACTIVE || record.status == STATUS_CLAIMED)
            && record.user_id.is_some()
    }
}

/// Log a storage failure behind a token endpoint answer and hide it.
pub(crate) fn storage_failure(error: Error) -> TokenError {
    tracing::error!(error = %error, "agent auth storage failed");
    TokenError::server_error()
}

/// Append one query parameter to an absolute or host-relative address.
pub(crate) fn append_parameter(base: &str, name: &str, value: &str) -> String {
    let separator = if base.contains('?') { '&' } else { '?' };
    let pair = url::form_urlencoded::Serializer::new(String::new())
        .append_pair(name, value)
        .finish();
    format!("{base}{separator}{pair}")
}

/// `bytes` random bytes from the operating system, as lowercase hex.
pub(crate) fn random_hex(bytes: usize) -> String {
    let mut buffer = vec![0_u8; bytes];
    rand::rngs::OsRng.fill_bytes(&mut buffer);
    crate::storage::hex_lower(&buffer)
}

/// A six-digit user code, uniform over `000000` to `999999`.
pub(crate) fn new_user_code() -> String {
    format!("{:06}", rand::rngs::OsRng.gen_range(0..1_000_000_u32))
}

/// Lowercase hex SHA-256 of a one-time value, the only form it is stored
/// in.
pub(crate) fn digest(value: &str) -> String {
    crate::authorization_server::token_digest(value)
}

/// The moment `lifetime` after `now`, refusing one no timestamp holds.
pub(crate) fn later(now: DateTime<Utc>, lifetime: Duration) -> Result<DateTime<Utc>> {
    chrono::Duration::from_std(lifetime)
        .ok()
        .and_then(|delta| now.checked_add_signed(delta))
        .ok_or_else(|| Error::InvalidInput {
            field: "lifetime".to_owned(),
            message: "is longer than a timestamp can hold".to_owned(),
        })
}

/// A timestamp as auth.md writes them: RFC 3339 with milliseconds, UTC.
pub(crate) fn timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// A normalized email address, or `None` for anything that is not one.
pub(crate) fn normalize_email(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 254
        || value.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return None;
    }
    let (local, domain) = value.split_once('@')?;
    if local.is_empty() || domain.contains('@') || !domain.contains('.') {
        return None;
    }
    if domain.starts_with('.') || domain.ends_with('.') {
        return None;
    }
    // The fold every other Magnetar lookup uses, so an address matches the
    // account the password plugin stored for it whatever its case.
    Some(crate::password::normalize_email(value))
}

/// Whether two addresses are the same once normalized as Magnetar stores
/// them.
pub(crate) fn same_email(left: &str, right: &str) -> bool {
    crate::password::normalize_email(left) == crate::password::normalize_email(right)
}

/// Headers that keep protocol answers out of caches (RFC 6749 section 5.1).
pub(crate) const NO_STORE: [(&str, &str); 2] =
    [("cache-control", "no-store"), ("pragma", "no-cache")];
/// Headers that keep the claim page's routes out of frames.
pub(crate) const NO_FRAMING: [(&str, &str); 2] = [
    ("x-frame-options", "DENY"),
    ("content-security-policy", "frame-ancestors 'none'"),
];

/// A JSON response with headers.
pub(crate) fn respond(status: u16, body: Option<Value>, headers: &[(&str, &str)]) -> WireResponse {
    let mut response = EffectResponse::ok();
    response.status = status;
    response.body = body;
    for (name, value) in headers {
        response
            .headers
            .insert((*name).to_owned(), (*value).to_owned());
    }
    WireResponse::from_effects(response)
}

/// An error of the identity and claim endpoints. The body carries
/// `error_description`, as `AUTH.md` shows it, and `message`, as the
/// service guide shows it, with the same text.
pub(crate) fn agent_error(status: u16, error: &str, description: &str) -> WireResponse {
    respond(
        status,
        Some(json!({
            "error": error,
            "error_description": description,
            "message": description,
        })),
        &NO_STORE,
    )
}

/// The `429 rate_limited` answer, with the time to wait.
pub(crate) fn rate_limited(retry_after: Duration) -> WireResponse {
    let seconds = retry_after.as_secs().max(1).to_string();
    respond(
        429,
        Some(json!({
            "error": "rate_limited",
            "error_description": "too many registrations; back off and retry",
            "message": "too many registrations; back off and retry",
        })),
        &[NO_STORE[0], NO_STORE[1], ("retry-after", seconds.as_str())],
    )
}

/// Answer a storage or limiter failure without its detail.
pub(crate) fn failure(error: &Error) -> WireResponse {
    tracing::error!(error = %error, "agent auth request failed");
    match error {
        Error::DependencyUnavailable { .. } => respond(
            503,
            Some(json!({"error": "temporarily_unavailable"})),
            &NO_STORE,
        ),
        _ => respond(500, Some(json!({"error": "server_error"})), &NO_STORE),
    }
}
