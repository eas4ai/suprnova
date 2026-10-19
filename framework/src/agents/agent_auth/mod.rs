//! Agent accounts under auth.md v0.6.0 (AGENT-007 to AGENT-011): the
//! Magnetar agent auth plugin, set up from the application's configuration
//! and mounted beside the OAuth server. [`AgentAuth`] documents what it
//! builds and mounts.

mod audit;
mod directory;
pub(crate) mod operator;
mod pages;
pub(crate) mod resolver;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use magnetar::agent_auth::{
    AgentAuthConfig, AgentAuthServices, AgentAuthStore, AgentRegistrationSummary,
    AgentUserDirectory, IdentityTypes, KeySetFetcher, KeySetPolicy, PreviousSigningKey,
    ProvidedIdentity, RefuseProvisioning, RegistrationLimit, SigningKey, SqlAgentAuthStore,
    TrustedIssuer,
};
use magnetar::authorization_server::{AuthorizationServer, HostResolver, PinnedHttpClient};
use magnetar::plugins::agent_auth::AgentAuthPlugin;
use sea_orm::DatabaseConnection;
use serde::Serialize;

use super::oauth::GrantScope;
use super::{AgentsConfig, CallLimit};
use crate::error::FrameworkError;
use crate::magnetar_integration::plugin_host::PluginMount;
use crate::routing::Router;

/// The environment variable that names the signing key's identifier.
pub(crate) const KEY_ID_VARIABLE: &str = "AGENT_AUTH_KEY_ID";
/// The environment variable that holds the signing key, a P-256 private key
/// in PEM form.
pub(crate) const PRIVATE_KEY_VARIABLE: &str = "AGENT_AUTH_PRIVATE_KEY";
/// The environment variable that names the previous signing key's
/// identifier, after a rotation.
pub(crate) const PREVIOUS_KEY_ID_VARIABLE: &str = "AGENT_AUTH_PREVIOUS_KEY_ID";
/// The environment variable that holds the previous signing key.
pub(crate) const PREVIOUS_PRIVATE_KEY_VARIABLE: &str = "AGENT_AUTH_PREVIOUS_PRIVATE_KEY";
/// The environment variable that says when the previous key stopped
/// signing, in RFC 3339.
pub(crate) const PREVIOUS_ROTATED_AT_VARIABLE: &str = "AGENT_AUTH_PREVIOUS_ROTATED_AT";

/// The agent auth route names that agents and providers call. They get no
/// session, no cookie and no CSRF check (AGENT-011); the two claim page
/// routes are browser routes.
const CLIENT_ROUTES: [&str; 3] = [
    "agent-auth.identity",
    "agent-auth.claim",
    "agent-auth.document",
];

/// The route that takes a provider's Security Event Tokens. The plugin
/// declares it only while ID-JAGs are accepted, so it is named a client
/// route only then.
const EVENTS_ROUTE: &str = "agent-auth.events";

/// An auth.md registration type (AGENT-007).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IdentityType {
    /// `anonymous`: an agent with no user identity registers itself. It
    /// holds the pre-claim scopes until a user claims it.
    Anonymous,
    /// `service_auth`: an agent names the user's email, and the user
    /// confirms with the code the agent hands them.
    ServiceAuth,
    /// `identity_assertion`: an agent presents an ID-JAG from a provider on
    /// the application's trust list.
    IdentityAssertion,
}

/// Creates a user for an ID-JAG that matches no user of the application,
/// or refuses (AGENT-008).
///
/// Agent auth asks only after the ID-JAG verified and matched no existing
/// delegation, verified email or verified phone. The user it names must
/// have the provider's verified email, verified: the identity assertion
/// agent auth signs for a bound user says its email is verified.
///
/// A provisioner must create a new user and never answer with one that
/// already exists. An existing account that holds the address was already
/// looked for, and it is bound only after its owner signs in and confirms
/// the claim; a provisioner that finds and returns it would bind it with no
/// ceremony.
///
/// ```rust,ignore
/// struct CreateUsers;
///
/// #[async_trait]
/// impl AgentUserProvisioner for CreateUsers {
///     async fn provision(&self, identity: &ProvidedIdentity) -> Result<Option<String>, FrameworkError> {
///         let Some(email) = &identity.verified_email else { return Ok(None) };
///         let user = User::create_verified(email).await?;
///         Ok(Some(user.id.to_string()))
///     }
/// }
/// ```
#[async_trait]
pub trait AgentUserProvisioner: Send + Sync {
    /// The identifier of a new user created for `identity`, with the
    /// provider's verified email marked verified; `None` refuses.
    async fn provision(
        &self,
        identity: &ProvidedIdentity,
    ) -> Result<Option<String>, FrameworkError>;
}

/// An agent registration bound to the signed-in user, for a
/// connected-agents page (AGENT-010).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentAccount {
    /// The registration's identifier, to revoke it by. Tools see it as
    /// `ToolCaller::registration().id()`.
    pub id: String,
    /// How the agent registered: `anonymous`, `service_auth` or
    /// `identity_assertion`.
    pub registration_type: String,
    /// The provider's display name from the application's trust list, for
    /// an ID-JAG registration. Never a name the provider sent.
    pub provider: Option<String>,
    /// What the registration may do, with the descriptions the claim page
    /// showed.
    pub scopes: Vec<GrantScope>,
    /// When the agent registered.
    pub created_at: DateTime<Utc>,
    /// When the user claimed it; `None` for an ID-JAG registration bound
    /// without a ceremony.
    pub claimed_at: Option<DateTime<Utc>>,
}

impl AgentAccount {
    /// The registration as agent auth's store summarizes it. Kept off the
    /// public surface: Magnetar's summary type is not one an application
    /// can name.
    pub(crate) fn from_summary(summary: AgentRegistrationSummary) -> Self {
        Self {
            id: summary.registration_id,
            registration_type: summary.registration_type,
            provider: summary.provider,
            scopes: summary
                .scopes
                .into_iter()
                .map(|scope| GrantScope {
                    name: scope.name,
                    description: scope.description,
                })
                .collect(),
            created_at: summary.created_at,
            claimed_at: summary.claimed_at,
        }
    }
}

/// A signed-in user claimed an agent registration (AGENT-009), so the
/// application can move what the agent created to that user.
///
/// It is delivered at least once. The claim's own transaction writes the
/// event to agent auth's outbox, so a committed claim always has it; it is
/// dispatched right after the commit, and an event whose dispatch failed or
/// whose process stopped first is dispatched by the maintenance pass `serve`
/// runs every five minutes and by `agents:prune`. A listener's error never
/// undoes the claim; it leaves the event for the next pass.
///
/// So the event can arrive more than once, after a crash or a retry, and a
/// listener must be idempotent, keyed on [`event_id`](Self::event_id): do
/// the move once per id, or make doing it again change nothing.
///
/// A listener that keeps failing gets twelve attempts, about an hour of
/// passes. The event then fails: it is logged once at error level, kept,
/// and delivered again only after `agents:prune --retry-failed` puts it
/// back, which an operator runs once the listener is fixed. Keep the
/// registration id on every row an agent creates (a tool reads it as
/// `ToolCaller::registration().id()`), so the move is a query by that id;
/// [`ConnectedAgents::agent_accounts`](super::ConnectedAgents::agent_accounts)
/// lists every registration bound to a user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentClaimed {
    /// The event's stable identifier, the same on every delivery of it.
    pub event_id: String,
    /// The registration the user claimed.
    pub registration_id: String,
    /// How the agent registered: `anonymous`, `service_auth` or
    /// `identity_assertion`.
    pub registration_type: String,
    /// The user who claimed it.
    pub user_id: String,
    /// The provider's display name from the trust list, for an ID-JAG
    /// registration claimed through the step-up ceremony.
    pub provider: Option<String>,
    /// The address the claim was confirmed from, when known.
    pub client_address: Option<String>,
}

impl crate::events::Event for AgentClaimed {
    fn event_name() -> &'static str {
        "AgentClaimed"
    }
}

/// Agent accounts under auth.md v0.6.0: the Magnetar agent auth plugin,
/// set up from the application's configuration (AGENT-007 to AGENT-011).
///
/// [`AgentsConfig::agent_auth`] turns it on beside the OAuth server, whose
/// token endpoint issues the agent tokens:
///
/// ```rust,ignore
/// use suprnova::agents::{AgentAuth, AgentsConfig, IdentityType, OAuthServer, TrustedIssuer};
///
/// pub fn register() -> Router {
///     Router::new()
///         .post("/posts", posts::store)
///         .agents(
///             AgentsConfig::new()
///                 .oauth_server(OAuthServer::new())
///                 .agent_auth(
///                     AgentAuth::new()
///                         .enable(IdentityType::IdentityAssertion)
///                         .trust(TrustedIssuer::new(
///                             "https://agents.acme.example",
///                             "https://agents.acme.example/.well-known/jwks.json",
///                             "Acme",
///                         ))
///                         .pre_claim_scopes([scope::POSTS_READ])
///                         .post_claim_scopes([scope::POSTS_READ, scope::POSTS_WRITE]),
///                 ),
///         )
/// }
/// ```
///
/// When the router is built, the framework adds agent auth to the OAuth
/// server and mounts it:
///
/// - **routes**: `POST /agent/identity`, `POST /agent/identity/claim`,
///   `POST /agent/event/notify` (only while ID-JAGs are accepted) and
///   `GET /auth.md`, which agents and providers call with no session and
///   no CSRF token; and the claim page's two routes,
///   `GET /agent/identity/claim/attempt` and
///   `POST /agent/identity/claim/complete`, which take the signed-in user's
///   session and the CSRF token. Every route bounds its body to 16 KiB;
/// - **grants**: the JWT bearer and claim grants on the OAuth server's token
///   endpoint, which issue no refresh token, and the `agent_auth` block in
///   its metadata;
/// - **the claim page** at `/agent/claim`: plain server-rendered HTML that
///   requires the signed-in user, lists every scope the agent will hold and
///   takes the code by a CSRF-protected form. The verification URI an agent
///   hands the user is this page, which sends a guest to sign in and back;
/// - **the resolver** that turns an agent token into an agent caller, asked
///   with the OAuth server's own, and refusing a token whose registration
///   was revoked or ended even if a grant revocation failed;
/// - **the connected-agents page** of the OAuth server, which gains an
///   "Agents" section listing and revoking the user's registrations;
/// - **audit**: one structured log record per auth.md audit event, a
///   [`MagnetarLifecycleEvent`](crate::magnetar_integration::engine::MagnetarLifecycleEvent)
///   carrying its attributes, and [`AgentClaimed`] for a claim, delivered
///   at least once through an outbox written in the claim's transaction;
/// - **rate limits** through the framework's rate limiter: 5 anonymous and
///   5 `service_auth` registrations an hour per client address and 100 per
///   application, 60 and 1000 for ID-JAGs;
/// - **maintenance**: every five minutes `serve` delivers claim events a
///   failed dispatch or a stopped process left, expires unclaimed
///   registrations past their window, prunes spent replay records and
///   deletes claim events delivered more than seven days ago; the
///   `agents:prune` command runs the same pass once and reports how many
///   claim events failed, and every node may run it. A claim event whose
///   listener refused it twelve times fails, is logged once at error level
///   and is never delivered again or pruned until
///   `agents:prune --retry-failed` puts it back with a fresh count;
/// - **storage**: the OAuth server's connection, or
///   [`connection`](Self::connection). Its four tables come from
///   [`AgentAuth::migrate`], or from the Magnetar default engine's
///   migrations.
///
/// # Defaults
///
/// Anonymous and `service_auth` registrations are on, ID-JAGs off until
/// [`enable`](Self::enable) and [`trust`](Self::trust). Before a claim an
/// anonymous registration holds **no scope** unless
/// [`pre_claim_scopes`](Self::pre_claim_scopes) names some; once bound to a
/// user a registration holds **every scope the application declared** with
/// `tool_scopes!` unless [`post_claim_scopes`](Self::post_claim_scopes)
/// names fewer. A tool still accepts agents only when it lists `agent` in
/// its callers.
///
/// # The signing key
///
/// Identity assertions are signed with a P-256 key the application
/// configures: [`signing_key`](Self::signing_key), or else
/// `AGENT_AUTH_KEY_ID` and `AGENT_AUTH_PRIVATE_KEY` (a PEM, where a literal
/// `\n` stands for a line break). `app agents:key` prints a new pair. After
/// a rotation the previous key keeps verifying what it signed until those
/// assertions expire: [`previous_signing_key`](Self::previous_signing_key),
/// or `AGENT_AUTH_PREVIOUS_KEY_ID`, `AGENT_AUTH_PREVIOUS_PRIVATE_KEY` and
/// `AGENT_AUTH_PREVIOUS_ROTATED_AT` (RFC 3339), so a rotation needs no code
/// change. A missing or unreadable key fails the build in every
/// environment: an ephemeral key would make nodes and restarts disagree.
/// After a key leaks, configure a new key first and leave the leaked one
/// out of the previous key, then run `app agents:revoke-all`: the command
/// builds the router, which needs a key, and revokes every registration and
/// token the leaked key could have reached.
///
/// # Replacing the claim page
///
/// [`custom_claim_page`](Self::custom_claim_page) serves no framework page
/// and sends users to the application's own. That page receives the
/// attempt in the `claim_attempt_token` query parameter. It must send a
/// guest to sign in and back to itself, address and query included, since
/// the verification URI goes straight to it. It reads the pending claim as
/// JSON from `GET /agent/identity/claim/attempt?claim_attempt_token=...`,
/// shows the provider and every scope from that JSON, and posts
/// `claim_attempt_token` and `user_code` with the CSRF token to
/// `POST /agent/identity/claim/complete`.
///
/// # Cross-origin requests
///
/// The claim routes answer with the signed-in user's email. An
/// application's CORS policy must not open them, or any `/agent` route, to
/// cross-origin requests with credentials.
///
/// # The audit trail
///
/// Records name the user's email (`claim.requested`) and text the agent
/// chose (`agent_platform`, `agent_context_id`). They are written as
/// fields, never into the message, but the channel holds email addresses:
/// keep it where personal data may be kept.
#[derive(Clone)]
pub struct AgentAuth {
    connection: Option<DatabaseConnection>,
    signing_key: Option<Result<SigningKey, String>>,
    previous_keys: Vec<Result<PreviousSigningKey, String>>,
    identity_types: IdentityTypes,
    trusted: Vec<TrustedIssuer>,
    pre_claim_scopes: Vec<String>,
    post_claim_scopes: Option<Vec<String>>,
    max_auth_age: Option<Duration>,
    registration_window: Option<Duration>,
    user_code_lifetime: Option<Duration>,
    max_user_code_failures: Option<u32>,
    assertion_lifetime: Option<Duration>,
    limits: Vec<(IdentityType, CallLimit, CallLimit)>,
    provisioner: Option<Arc<dyn AgentUserProvisioner>>,
    directory: Option<Arc<dyn AgentUserDirectory>>,
    route_prefix: String,
    document_path: String,
    claim_path: String,
    default_claim_page: bool,
    service_name: Option<String>,
    audit_channel: Option<String>,
    key_fetch: Option<(Arc<dyn HostResolver>, Arc<dyn PinnedHttpClient>)>,
    key_cache: Option<(Duration, Duration)>,
    event_delivery_timeout: Option<Duration>,
}

impl std::fmt::Debug for AgentAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Key identifiers only: the keys never print.
        let kid = |key: &Result<SigningKey, String>| match key {
            Ok(key) => key.kid().to_owned(),
            Err(_) => "<unreadable>".to_owned(),
        };
        f.debug_struct("AgentAuth")
            .field("signing_key", &self.signing_key.as_ref().map(kid))
            .field(
                "previous_keys",
                &self
                    .previous_keys
                    .iter()
                    .map(|key| match key {
                        Ok(previous) => previous.key.kid().to_owned(),
                        Err(_) => "<unreadable>".to_owned(),
                    })
                    .collect::<Vec<_>>(),
            )
            .field("identity_types", &self.identity_types)
            .field("trusted", &self.trusted)
            .field("pre_claim_scopes", &self.pre_claim_scopes)
            .field("post_claim_scopes", &self.post_claim_scopes)
            .field("route_prefix", &self.route_prefix)
            .field("document_path", &self.document_path)
            .field("claim_path", &self.claim_path)
            .field("default_claim_page", &self.default_claim_page)
            .finish_non_exhaustive()
    }
}

impl Default for AgentAuth {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentAuth {
    /// Agent auth with every default (see the type's documentation): the
    /// signing key from the environment, anonymous and `service_auth`
    /// registrations on, no scope before a claim and every declared scope
    /// after, the routes under `/agent`, the document at `/auth.md` and the
    /// framework's claim page at `/agent/claim`.
    pub fn new() -> Self {
        Self {
            connection: None,
            signing_key: None,
            previous_keys: Vec::new(),
            identity_types: IdentityTypes::default(),
            trusted: Vec::new(),
            pre_claim_scopes: Vec::new(),
            post_claim_scopes: None,
            max_auth_age: None,
            registration_window: None,
            user_code_lifetime: None,
            max_user_code_failures: None,
            assertion_lifetime: None,
            limits: Vec::new(),
            provisioner: None,
            directory: None,
            route_prefix: "/agent".to_owned(),
            document_path: "/auth.md".to_owned(),
            claim_path: "/agent/claim".to_owned(),
            default_claim_page: true,
            service_name: None,
            audit_channel: None,
            key_fetch: None,
            key_cache: None,
            event_delivery_timeout: None,
        }
    }

    /// Keep registrations, claim ceremonies and replay records through
    /// `connection` rather than the OAuth server's connection.
    pub fn connection(mut self, connection: DatabaseConnection) -> Self {
        self.connection = Some(connection);
        self
    }

    /// Sign identity assertions with the P-256 private key `pem` (PKCS#8 or
    /// SEC1 PEM; a literal `\n` stands for a line break), named `kid` in
    /// every assertion, instead of the key in the environment. A key that
    /// does not read fails the build.
    pub fn signing_key(mut self, kid: impl Into<String>, pem: impl Into<String>) -> Self {
        self.signing_key = Some(read_key(kid.into(), &pem.into()));
        self
    }

    /// Keep verifying the assertions `pem` signed before it was rotated out
    /// at `rotated_out_at`, until the longest of them expires. It signs
    /// nothing new. Naming one here leaves the environment's previous key
    /// unread.
    pub fn previous_signing_key(
        mut self,
        kid: impl Into<String>,
        pem: impl Into<String>,
        rotated_out_at: DateTime<Utc>,
    ) -> Self {
        self.previous_keys.push(
            read_key(kid.into(), &pem.into())
                .map(|key| PreviousSigningKey::new(key, rotated_out_at)),
        );
        self
    }

    /// Accept registrations of type `identity_type`. ID-JAGs also need a
    /// provider on the trust list.
    pub fn enable(mut self, identity_type: IdentityType) -> Self {
        self.set_type(identity_type, true);
        self
    }

    /// Refuse registrations of type `identity_type`: the identity endpoint
    /// answers its `*_not_enabled` code and `/auth.md` leaves it out.
    pub fn disable(mut self, identity_type: IdentityType) -> Self {
        self.set_type(identity_type, false);
        self
    }

    /// Accept ID-JAGs from `issuer`. Its display name is the one the claim
    /// page shows; a name the provider sends is never shown.
    pub fn trust(mut self, issuer: TrustedIssuer) -> Self {
        self.trusted.push(issuer);
        self
    }

    /// The scopes an anonymous registration holds before a user claims it:
    /// none unless named. Each must be declared with `tool_scopes!` and be
    /// among the post-claim scopes.
    pub fn pre_claim_scopes<I>(mut self, scopes: I) -> Self
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        self.pre_claim_scopes = names(scopes);
        self
    }

    /// The scopes a registration holds once bound to a user: every scope
    /// the application declared with `tool_scopes!` unless named. The claim
    /// page lists each.
    pub fn post_claim_scopes<I>(mut self, scopes: I) -> Self
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        self.post_claim_scopes = Some(names(scopes));
        self
    }

    /// The oldest user sign-in an ID-JAG's `auth_time` may name: one hour
    /// unless set. An older or missing one answers 401 `login_required`.
    pub fn max_auth_age(mut self, age: Duration) -> Self {
        self.max_auth_age = Some(age);
        self
    }

    /// How long a registration waits for its claim: 24 hours unless set.
    pub fn registration_window(mut self, window: Duration) -> Self {
        self.registration_window = Some(window);
        self
    }

    /// How long a user code works: ten minutes unless set.
    pub fn user_code_lifetime(mut self, lifetime: Duration) -> Self {
        self.user_code_lifetime = Some(lifetime);
        self
    }

    /// How many wrong user codes one claim attempt survives: 5 unless set.
    pub fn max_user_code_failures(mut self, failures: u32) -> Self {
        self.max_user_code_failures = Some(failures);
        self
    }

    /// How long an identity assertion can be exchanged: 30 days unless
    /// set. An unclaimed registration's assertion ends with its window.
    pub fn assertion_lifetime(mut self, lifetime: Duration) -> Self {
        self.assertion_lifetime = Some(lifetime);
        self
    }

    /// The registration budgets of `identity_type`: `per_address` for one
    /// client address, checked first, and `per_application` for the whole
    /// application. Unless set: 5 and 100 an hour for anonymous and
    /// `service_auth` registrations, 60 and 1000 for ID-JAGs.
    pub fn registration_limit(
        mut self,
        identity_type: IdentityType,
        per_address: CallLimit,
        per_application: CallLimit,
    ) -> Self {
        self.limits
            .push((identity_type, per_address, per_application));
        self
    }

    /// Create users for ID-JAGs that match none through `provisioner`.
    /// Without one such an ID-JAG is refused.
    pub fn provision_users(mut self, provisioner: Arc<dyn AgentUserProvisioner>) -> Self {
        self.provisioner = Some(provisioner);
        self
    }

    /// Find the application's users through `directory` instead of the
    /// default guard's user provider (or the Magnetar engine, where it owns
    /// the default guard's sessions), for an application that matches
    /// ID-JAGs by verified phone, say.
    ///
    /// The directory's `auth_epoch` must be the epoch the default guard's
    /// sessions carry, or every claim answers `login_required` and every
    /// bound agent token stops working.
    pub fn user_directory(mut self, directory: Arc<dyn AgentUserDirectory>) -> Self {
        self.directory = Some(directory);
        self
    }

    /// Mount the agent routes under `prefix` instead of `/agent`. A prefix
    /// without a leading `/` gets one.
    pub fn route_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.route_prefix = leading_slash(prefix.into());
        self
    }

    /// Serve the instructions an agent reads at `path` instead of
    /// `/auth.md`.
    pub fn document_path(mut self, path: impl Into<String>) -> Self {
        self.document_path = leading_slash(path.into());
        self
    }

    /// Serve the framework's claim page at `path` instead of
    /// `/agent/claim`.
    pub fn claim_page(mut self, path: impl Into<String>) -> Self {
        self.claim_path = leading_slash(path.into());
        self.default_claim_page = true;
        self
    }

    /// Send users to the application's own claim page at `path`, and
    /// serve no default page. The type's "Replacing the claim page"
    /// section says what the page must do.
    pub fn custom_claim_page(mut self, path: impl Into<String>) -> Self {
        self.claim_path = leading_slash(path.into());
        self.default_claim_page = false;
        self
    }

    /// The name `/auth.md` gives the application: its name (`APP_NAME`)
    /// unless set.
    pub fn service_name(mut self, name: impl Into<String>) -> Self {
        self.service_name = Some(name.into());
        self
    }

    /// Write the audit records to the log channel `name` instead of the
    /// default channel. The channel must exist when the router is built.
    pub fn audit_log_channel(mut self, name: impl Into<String>) -> Self {
        self.audit_channel = Some(name.into());
        self
    }

    /// Fetch providers' key sets through `resolver` and `client` instead of
    /// the system resolver and Magnetar's pinned HTTPS client. A test gives
    /// its own, so no test reaches the network.
    pub fn key_set_fetch(
        mut self,
        resolver: Arc<dyn HostResolver>,
        client: Arc<dyn PinnedHttpClient>,
    ) -> Self {
        self.key_fetch = Some((resolver, client));
        self
    }

    /// Reuse a fetched key set for at least `floor` and at most `ceiling`,
    /// whatever its `Cache-Control` says: 10 minutes and 24 hours unless
    /// set. An unknown key id refetches once, at most once a minute.
    pub fn key_set_cache(mut self, floor: Duration, ceiling: Duration) -> Self {
        self.key_cache = Some((floor, ceiling));
        self
    }

    /// How long one delivery of a claim event may take before it counts as
    /// a refused attempt with "the delivery timed out": 60 seconds unless
    /// set. A claim confirmation waits at most this long for its event's
    /// first delivery, and a slow listener cannot stall the maintenance
    /// pass. Each delivery's lease is twice as long.
    pub fn event_delivery_timeout(mut self, timeout: Duration) -> Self {
        self.event_delivery_timeout = Some(timeout);
        self
    }

    /// Create agent auth's tables on `connection` if they do not exist:
    /// `auth_agent_registrations`, `auth_agent_grants`,
    /// `auth_agent_replays` and `auth_agent_events`, the outbox of claim
    /// events. Safe to run on every boot. The Magnetar default engine's
    /// migrations create them too.
    ///
    /// # Errors
    ///
    /// When the database refuses a statement.
    pub async fn migrate(connection: &DatabaseConnection) -> Result<(), FrameworkError> {
        magnetar::agent_auth::migrate(connection)
            .await
            .map_err(|error| {
                FrameworkError::internal(format!("create agent auth's tables: {error}"))
            })
    }

    fn set_type(&mut self, identity_type: IdentityType, on: bool) {
        match identity_type {
            IdentityType::Anonymous => self.identity_types.anonymous = on,
            IdentityType::ServiceAuth => self.identity_types.service_auth = on,
            IdentityType::IdentityAssertion => self.identity_types.identity_assertion = on,
        }
    }

    /// Add agent auth to `server`, the OAuth server built for the same
    /// router, whose tables are on `connection`.
    pub(crate) fn install(
        self,
        server: AuthorizationServer,
        connection: &DatabaseConnection,
    ) -> Result<(Arc<AuthorizationServer>, BuiltAgentAuth), FrameworkError> {
        if !self.trusted.is_empty() && !self.identity_types.identity_assertion {
            return Err(FrameworkError::internal(
                "AgentAuth::trust names a provider, and ID-JAGs are off: enable \
                 IdentityType::IdentityAssertion, or remove the trust list, so a configured \
                 provider is never silently ignored",
            ));
        }
        if let Some(channel) = &self.audit_channel {
            crate::logging::Log::channel(channel).map_err(|error| {
                FrameworkError::internal(format!(
                    "agent auth's audit log channel '{channel}' cannot be used: {error}"
                ))
            })?;
        }
        let (signing_key, previous_keys) = self.signing_keys()?;
        let declared: Vec<String> = server
            .config()
            .scopes
            .iter()
            .map(|scope| scope.name.clone())
            .collect();
        let mut config = AgentAuthConfig::new(signing_key);
        config.previous_signing_keys = previous_keys;
        config.route_prefix = self.route_prefix.clone();
        config.skill_path = self.document_path.clone();
        config.claim_page_url = self.claim_path.clone();
        // The verification URI is the claim page itself, which sends a
        // guest to sign in: a sign-in page that sends a signed-in user home
        // cannot lose the claim.
        config.direct_claim_page = true;
        config.service_name = self
            .service_name
            .clone()
            .unwrap_or_else(super::metadata::app_name);
        config.identity_types = self.identity_types;
        config.trusted_issuers = self.trusted.clone();
        config.pre_claim_scopes = self.pre_claim_scopes.clone();
        config.post_claim_scopes = self.post_claim_scopes.clone().unwrap_or(declared);
        if let Some(age) = self.max_auth_age {
            config.id_jag_max_auth_age = age;
        }
        if let Some(window) = self.registration_window {
            config.registration_window = window;
        }
        if let Some(lifetime) = self.user_code_lifetime {
            config.user_code_lifetime = lifetime;
        }
        if let Some(failures) = self.max_user_code_failures {
            config.max_user_code_failures = failures;
        }
        if let Some(lifetime) = self.assertion_lifetime {
            config.assertion_lifetime = lifetime;
        }
        if let Some(timeout) = self.event_delivery_timeout {
            config.event_delivery_timeout = timeout;
        }
        for (identity_type, per_address, per_application) in &self.limits {
            let limit = RegistrationLimit {
                per_address: policy(*per_address),
                per_application: policy(*per_application),
            };
            match identity_type {
                IdentityType::Anonymous => config.limits.anonymous = limit,
                IdentityType::ServiceAuth => config.limits.service_auth = limit,
                IdentityType::IdentityAssertion => config.limits.identity_assertion = limit,
            }
        }
        let connection = self
            .connection
            .clone()
            .unwrap_or_else(|| connection.clone());
        let store: Arc<dyn AgentAuthStore> = Arc::new(SqlAgentAuthStore::new(connection.clone()));
        let users: Arc<dyn AgentUserDirectory> = self
            .directory
            .clone()
            .unwrap_or_else(|| Arc::new(directory::HostAgentUsers));
        let provisioning: Arc<dyn magnetar::agent_auth::ProvisioningPolicy> =
            match &self.provisioner {
                Some(provisioner) => Arc::new(directory::ProvisionWith::new(
                    Arc::clone(provisioner),
                    Arc::clone(&users),
                )),
                None => Arc::new(RefuseProvisioning),
            };
        let mut key_policy = KeySetPolicy::default();
        if let Some((floor, ceiling)) = self.key_cache {
            key_policy.min_ttl = floor;
            key_policy.max_ttl = ceiling;
        }
        let key_sets = Arc::new(match &self.key_fetch {
            Some((resolver, client)) => {
                KeySetFetcher::new(Arc::clone(resolver), Arc::clone(client), key_policy)
            }
            None => KeySetFetcher::system(key_policy),
        });
        let events = Arc::new(audit::AuditSink::new(
            self.audit_channel.clone(),
            self.trusted.clone(),
        ));
        let services = AgentAuthServices {
            store: Arc::clone(&store),
            users,
            provisioning,
            limiter: Arc::new(crate::magnetar_integration::abuse_limiter::AppAbuseLimiter),
            key_sets,
            events,
        };
        let (server, engine) = magnetar::agent_auth::AgentAuth::install(server, config, services)
            .map_err(|error| {
            FrameworkError::internal(format!("agent auth's configuration is invalid: {error}"))
        })?;
        Ok((
            server,
            BuiltAgentAuth {
                mounted: Arc::new(MountedAgentAuth { engine, store }),
                setup: self,
                connection,
            },
        ))
    }

    /// The signing key and the previous keys: the builder's, or else the
    /// environment's.
    fn signing_keys(&self) -> Result<(SigningKey, Vec<PreviousSigningKey>), FrameworkError> {
        let current = match &self.signing_key {
            Some(key) => key.clone().map_err(|error| {
                FrameworkError::internal(format!(
                    "AgentAuth::signing_key is not a P-256 private key in PEM form: {error}"
                ))
            })?,
            None => key_from_environment()?,
        };
        let previous = if self.previous_keys.is_empty() {
            previous_key_from_environment()?.into_iter().collect()
        } else {
            self.previous_keys
                .iter()
                .cloned()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| {
                    FrameworkError::internal(format!(
                        "AgentAuth::previous_signing_key is not a P-256 private key in PEM \
                         form: {error}"
                    ))
                })?
        };
        Ok((current, previous))
    }
}

impl AgentsConfig {
    /// Serve agent accounts under auth.md v0.6.0 (AGENT-007 to AGENT-011),
    /// set up as [`AgentAuth`] describes. Agent auth issues its tokens on
    /// the OAuth server's token endpoint, so it needs
    /// [`oauth_server`](Self::oauth_server) too; without it the build fails.
    pub fn agent_auth(mut self, agent_auth: AgentAuth) -> Self {
        self.agent_auth = Some(agent_auth);
        self
    }
}

/// What agent auth runs on once installed, shared by the router, every
/// request it routes, the resolver and the pages.
pub(crate) struct MountedAgentAuth {
    /// The Magnetar engine.
    pub(crate) engine: Arc<magnetar::agent_auth::AgentAuth>,
    /// The store the engine writes, which the resolver reads a token's
    /// registration from.
    pub(crate) store: Arc<dyn AgentAuthStore>,
}

/// Agent auth installed on one router's OAuth server, ready to mount.
pub(crate) struct BuiltAgentAuth {
    mounted: Arc<MountedAgentAuth>,
    setup: AgentAuth,
    connection: DatabaseConnection,
}

impl BuiltAgentAuth {
    /// The engine and its store.
    pub(crate) fn mounted(&self) -> Arc<MountedAgentAuth> {
        Arc::clone(&self.mounted)
    }

    /// Mount the plugin's routes and the default claim page on `router`.
    /// The claim page signs a guest in at `sign_in_url` with the return
    /// address in `return_to_parameter`, as the OAuth server's pages do,
    /// and links the connected-agents page at `connected_path`.
    pub(crate) fn mount(
        self,
        router: Router,
        sign_in_url: String,
        return_to_parameter: String,
        connected_path: Option<String>,
    ) -> Result<Router, FrameworkError> {
        let engine = Arc::clone(&self.mounted.engine);
        let mut mount = PluginMount::new(
            Arc::new(AgentAuthPlugin::new(Arc::clone(&engine))),
            self.connection,
        )
        .max_body_bytes(engine.config().max_request_bytes);
        for name in CLIENT_ROUTES {
            mount = mount.client_route(name);
        }
        if engine.config().identity_types.identity_assertion {
            mount = mount.client_route(EVENTS_ROUTE);
        }
        let mut router = router.try_mount_magnetar_plugin(mount)?;
        if self.setup.default_claim_page {
            router = pages::mount_claim(
                router,
                Arc::new(pages::ClaimPage {
                    engine,
                    path: self.setup.claim_path.clone(),
                    sign_in_url,
                    return_to_parameter,
                    connected_path,
                }),
            )?;
        }
        Ok(router)
    }
}

impl Router {
    /// The agent auth engine this router's OAuth server carries, for the
    /// operator commands and the maintenance pass.
    pub(crate) fn agent_auth(&self) -> Option<Arc<magnetar::agent_auth::AgentAuth>> {
        self.agents
            .oauth
            .as_ref()
            .and_then(|mounted| mounted.agent.as_ref())
            .map(|agent| Arc::clone(&agent.engine))
    }
}

/// A key read from `pem`, a literal `\n` standing for a line break, as an
/// environment file writes one.
fn read_key(kid: String, pem: &str) -> Result<SigningKey, String> {
    SigningKey::from_pem(kid, &pem.replace("\\n", "\n")).map_err(|error| error.to_string())
}

/// The signing key the environment names.
fn key_from_environment() -> Result<SigningKey, FrameworkError> {
    let (Some(kid), Some(pem)) = (variable(KEY_ID_VARIABLE), variable(PRIVATE_KEY_VARIABLE)) else {
        return Err(FrameworkError::internal(format!(
            "agent auth signs identity assertions with a key the application configures, and \
             none is set: set {KEY_ID_VARIABLE} and {PRIVATE_KEY_VARIABLE} (a P-256 private key \
             in PEM form; `app agents:key` prints a new pair), or call AgentAuth::signing_key"
        )));
    };
    read_key(kid, &pem).map_err(|error| {
        FrameworkError::internal(format!(
            "{PRIVATE_KEY_VARIABLE} is not a P-256 private key in PEM form ({error}); \
             `app agents:key` prints a new pair"
        ))
    })
}

/// The previous signing key the environment names, if it names one.
fn previous_key_from_environment() -> Result<Option<PreviousSigningKey>, FrameworkError> {
    let kid = variable(PREVIOUS_KEY_ID_VARIABLE);
    let pem = variable(PREVIOUS_PRIVATE_KEY_VARIABLE);
    let rotated = variable(PREVIOUS_ROTATED_AT_VARIABLE);
    match (kid, pem, rotated) {
        (None, None, None) => Ok(None),
        (Some(kid), Some(pem), Some(rotated)) => {
            let rotated_out_at = DateTime::parse_from_rfc3339(rotated.trim())
                .map_err(|error| {
                    FrameworkError::internal(format!(
                        "{PREVIOUS_ROTATED_AT_VARIABLE} is not an RFC 3339 time: {error}"
                    ))
                })?
                .with_timezone(&Utc);
            let key = read_key(kid, &pem).map_err(|error| {
                FrameworkError::internal(format!(
                    "{PREVIOUS_PRIVATE_KEY_VARIABLE} is not a P-256 private key in PEM form \
                     ({error})"
                ))
            })?;
            Ok(Some(PreviousSigningKey::new(key, rotated_out_at)))
        }
        _ => Err(FrameworkError::internal(format!(
            "a previous agent auth signing key needs all of {PREVIOUS_KEY_ID_VARIABLE}, \
             {PREVIOUS_PRIVATE_KEY_VARIABLE} and {PREVIOUS_ROTATED_AT_VARIABLE}"
        ))),
    }
}

/// An environment variable's value, when it is set and not blank.
fn variable(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

/// The budget `limit` describes.
fn policy(limit: CallLimit) -> magnetar::plugin::AbusePolicy {
    magnetar::plugin::AbusePolicy {
        max_requests: limit.max_calls,
        window: limit.window,
    }
}

fn names<I>(scopes: I) -> Vec<String>
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    scopes
        .into_iter()
        .map(|scope| scope.as_ref().to_owned())
        .collect()
}

/// `path` with a leading `/`.
fn leading_slash(path: String) -> String {
    if path.starts_with('/') {
        path
    } else {
        format!("/{path}")
    }
}
