//! Persistence boundary for registered clients, grants and their tokens.
//!
//! A grant is one user's approval of one client for one resource and a set
//! of scopes. Every access and refresh token belongs to exactly one grant,
//! so ending a grant ends all of its tokens at once: refresh-token reuse,
//! revocation of a refresh token, an epoch change and a user's own
//! revocation all work on the grant. Tokens are stored only as SHA-256
//! digests; no method here ever receives a plaintext token.

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::Result;

/// A client registered through dynamic client registration (RFC 7591).
///
/// Clients identified by a client metadata document are never stored: the
/// document is their registration, fetched and cached on demand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisteredClient {
    /// The opaque identifier the registration endpoint minted.
    pub client_id: String,
    /// The name the client registered, shown on the approval page.
    pub client_name: Option<String>,
    /// The registered redirect URIs, compared exactly at authorization.
    pub redirect_uris: Vec<String>,
    /// The grant types the client registered.
    pub grant_types: Vec<String>,
    /// The scopes the client limited itself to, when it named any.
    pub scope: Option<Vec<String>>,
    /// The client's home page, when it gave one.
    pub client_uri: Option<String>,
    /// The client's logo, when it gave one.
    pub logo_uri: Option<String>,
    /// When the registration was created.
    pub created_at: DateTime<Utc>,
}

/// One grant: a user's (or, for extension grants, an agent's) approval of
/// one client for one resource and scope set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrantRecord {
    /// Generated grant identifier.
    pub id: String,
    /// The grant family, [`USER_GRANT_KIND`] for the authorization code
    /// grant; extension grants name their own.
    pub kind: String,
    /// The user the grant acts for, absent for an agent registration that
    /// no user has claimed.
    pub user_id: Option<String>,
    /// The client (or agent registration) the grant was issued to.
    pub client_id: String,
    /// The client's display name when the grant was issued.
    pub client_name: String,
    /// The host the approval page showed for the client.
    pub client_host: String,
    /// The approved scopes.
    pub scopes: Vec<String>,
    /// The RFC 8707 resource every token of the grant is bound to.
    pub resource: String,
    /// The user's authentication epoch when the grant was issued. A grant
    /// stops working once the user's epoch moves on.
    pub auth_epoch: Option<u64>,
    /// The kind of every access token the grant mints, kept so a refresh
    /// mints the kind the grant started with.
    pub access_kind: String,
    /// The lifetime of every access token the grant mints, when the issuer
    /// named one. `None` follows the server's configured lifetime, so an
    /// operator who shortens it shortens existing grants' next tokens too.
    pub access_lifetime: Option<Duration>,
    /// When the grant was issued.
    pub created_at: DateTime<Utc>,
    /// When one of the grant's access tokens was last verified.
    pub last_used_at: Option<DateTime<Utc>>,
    /// When the grant was revoked, if it was.
    pub revoked_at: Option<DateTime<Utc>>,
}

/// The kind of every grant the authorization code flow issues.
pub const USER_GRANT_KIND: &str = "authorization_code";

/// The kind of an access token the authorization code and refresh grants
/// issue.
pub const ACCESS_TOKEN_KIND: &str = "access";

/// The kind of a refresh token. Every other kind is an access token kind.
pub const REFRESH_TOKEN_KIND: &str = "refresh";

/// One stored token, never its plaintext.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenRecord {
    /// Generated token identifier.
    pub id: String,
    /// The grant the token belongs to.
    pub grant_id: String,
    /// [`REFRESH_TOKEN_KIND`] or an access token kind.
    pub kind: String,
    /// Lowercase hex SHA-256 digest of the token.
    pub token_hash: String,
    /// The scopes this token carries, a subset of the grant's.
    pub scopes: Vec<String>,
    /// When the token stops working.
    pub expires_at: DateTime<Utc>,
    /// When a refresh token was exchanged; a second exchange is reuse.
    pub used_at: Option<DateTime<Utc>>,
    /// When the token was revoked, if it was.
    pub revoked_at: Option<DateTime<Utc>>,
    /// When the token was issued.
    pub created_at: DateTime<Utc>,
}

/// A token to insert: its digest and claims, never its plaintext.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewTokenRecord {
    /// Generated token identifier.
    pub id: String,
    /// [`REFRESH_TOKEN_KIND`] or an access token kind.
    pub kind: String,
    /// Lowercase hex SHA-256 digest of the token.
    pub token_hash: String,
    /// The scopes the token carries.
    pub scopes: Vec<String>,
    /// When the token stops working.
    pub expires_at: DateTime<Utc>,
}

/// A short-lived step of the authorization flow: a pending authorization
/// waiting for the user's decision, or an issued authorization code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizationRecord {
    /// The pending authorization's identifier, or the digest of the code;
    /// a code itself is never stored.
    pub id: String,
    /// `pending` or `code`.
    pub kind: String,
    /// The step's state, moved only by a compare-and-set.
    pub state: String,
    /// The user the step belongs to.
    pub user_id: String,
    /// What the step stands for, as JSON.
    pub payload: String,
    /// When the step lapses.
    pub expires_at: DateTime<Utc>,
    /// When the step was created, the order trimming keeps the newest by.
    pub created_at: DateTime<Utc>,
}

/// Storage for the authorization server.
///
/// Implementations must make [`AuthorizationStore::rotate_refresh_token`]
/// a single-winner operation in the database itself: two requests that
/// present one refresh token at the same moment both read it as unused, so
/// a read followed by a write lets both rotate it.
#[async_trait]
pub trait AuthorizationStore: Send + Sync {
    /// Store a dynamically registered client.
    async fn insert_client(&self, client: RegisteredClient) -> Result<()>;
    /// Find a dynamically registered client.
    async fn find_client(&self, client_id: &str) -> Result<Option<RegisteredClient>>;
    /// Create a grant together with its first tokens, all or nothing.
    async fn create_grant(&self, grant: GrantRecord, tokens: Vec<NewTokenRecord>) -> Result<()>;
    /// Find a token by its digest, whatever its state.
    async fn find_token(&self, token_hash: &str) -> Result<Option<TokenRecord>>;
    /// Find a grant, whatever its state.
    async fn find_grant(&self, grant_id: &str) -> Result<Option<GrantRecord>>;
    /// Mark the refresh token used and insert its replacements, all or
    /// nothing, only when the token is still unused, unrevoked and
    /// unexpired and its grant is not revoked. Returns whether this call
    /// won; `false` means another request used or revoked it first.
    async fn rotate_refresh_token(
        &self,
        refresh_token_id: &str,
        grant_id: &str,
        at: DateTime<Utc>,
        replacements: Vec<NewTokenRecord>,
    ) -> Result<bool>;
    /// Revoke one token. Returns whether a live token was revoked.
    async fn revoke_token(&self, token_id: &str, at: DateTime<Utc>) -> Result<bool>;
    /// Add tokens to an existing grant, all or nothing, only while the
    /// grant is not revoked; the grant row is locked before any token row,
    /// the order every write path takes. Returns whether they were added.
    /// An extension that keeps one grant per party (agent auth) mints its
    /// later tokens this way. Stores that predate it answer
    /// [`crate::Error::DependencyUnavailable`].
    async fn add_tokens(
        &self,
        grant_id: &str,
        tokens: Vec<NewTokenRecord>,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        let _ = (grant_id, tokens, at);
        Err(crate::Error::DependencyUnavailable {
            dependency: "authorization store".to_owned(),
            message: "this store cannot add tokens to an existing grant".to_owned(),
        })
    }
    /// Revoke a grant and every one of its tokens. Returns whether the
    /// grant was live.
    async fn revoke_grant(&self, grant_id: &str, at: DateTime<Utc>) -> Result<bool>;
    /// Revoke every live grant of a user and their tokens. Returns how many
    /// grants were revoked.
    async fn revoke_user_grants(&self, user_id: &str, at: DateTime<Utc>) -> Result<u64>;
    /// Record that one of the grant's access tokens was just verified.
    async fn touch_grant(&self, grant_id: &str, at: DateTime<Utc>) -> Result<()>;
    /// List a user's unrevoked grants that still hold a live token, newest
    /// first.
    async fn list_user_grants(&self, user_id: &str, now: DateTime<Utc>)
    -> Result<Vec<GrantRecord>>;
    /// Store a pending authorization or a code, and drop steps that lapsed.
    async fn create_authorization(&self, record: AuthorizationRecord) -> Result<()>;
    /// Find a step that has not lapsed.
    async fn find_authorization(
        &self,
        id: &str,
        kind: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<AuthorizationRecord>>;
    /// Move a step from `expected` to `next`. With `live_at`, only a step
    /// that has not lapsed by then moves; with `None`, a lapsed one moves
    /// too, for the steps that finish what an earlier, checked step began.
    /// Returns whether this call won; only one concurrent caller can.
    async fn transition_authorization(
        &self,
        id: &str,
        kind: &str,
        expected: &str,
        next: &str,
        live_at: Option<DateTime<Utc>>,
    ) -> Result<bool>;
    /// Delete a step that has not lapsed. Returns whether this call deleted
    /// it; only one concurrent caller can.
    async fn consume_authorization(&self, id: &str, kind: &str, now: DateTime<Utc>)
    -> Result<bool>;
    /// Keep only the `keep` newest steps of one kind for a user, `newest`
    /// always among them, and delete the rest. Newest means latest
    /// `created_at`, ties broken by identifier. Returns how many went.
    async fn trim_authorizations(
        &self,
        user_id: &str,
        kind: &str,
        keep: u64,
        newest: &str,
    ) -> Result<u64>;
    /// Delete lapsed steps and tokens that expired before `now`, then grants
    /// that ended before `grant_cutoff`: those revoked before it, and those
    /// left with no token whose last activity is older than it. Returns how
    /// many rows went.
    async fn prune_expired(&self, now: DateTime<Utc>, grant_cutoff: DateTime<Utc>) -> Result<u64>;
}
