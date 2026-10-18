//! Persistence boundary for agent registrations, their claim ceremonies,
//! the authorization server grants issued to them and the replay records
//! of provider tokens.
//!
//! Every one-time value (claim token, claim attempt token, user code) is
//! stored only as a SHA-256 digest, and every step that spends one is a
//! conditional write the database lets exactly one caller win: a read
//! followed by a write would let two concurrent requests both pass.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::Result;

/// The kind of a registration, as auth.md names it in `registration_type`.
pub const ANONYMOUS: &str = "anonymous";
/// A registration made from the user's email.
pub const SERVICE_AUTH: &str = "service_auth";
/// A registration made from a provider's ID-JAG.
pub const IDENTITY_ASSERTION: &str = "identity_assertion";

/// An anonymous registration no user has claimed yet: it holds the
/// pre-claim scopes.
pub const STATUS_UNCLAIMED: &str = "unclaimed";
/// A registration waiting for its claim ceremony before it holds anything:
/// a `service_auth` one, or an ID-JAG that matched an existing account by
/// email or phone only.
pub const STATUS_PENDING: &str = "pending";
/// An ID-JAG registration bound to a user without a ceremony: the
/// delegation was known, or the provisioning policy created the user.
pub const STATUS_ACTIVE: &str = "active";
/// A registration a signed-in user claimed with the user code.
pub const STATUS_CLAIMED: &str = "claimed";
/// A registration whose window passed before anyone claimed it.
pub const STATUS_EXPIRED: &str = "expired";
/// A revoked registration.
pub const STATUS_REVOKED: &str = "revoked";

/// One agent registration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistrationRecord {
    /// The registration identifier, `reg_` and 32 hex digits. Identity
    /// assertions carry it as `sub`.
    pub id: String,
    /// [`ANONYMOUS`], [`SERVICE_AUTH`] or [`IDENTITY_ASSERTION`].
    pub kind: String,
    /// One of the `STATUS_*` values.
    pub status: String,
    /// The user the registration is bound to, once it is.
    pub user_id: Option<String>,
    /// The email the claim is bound to: only a user signed in with it can
    /// complete the ceremony.
    pub claim_email: Option<String>,
    /// The user an ID-JAG step-up matched: only that user can complete it.
    pub claim_user_id: Option<String>,
    /// Digest of the claim token, until the token is spent.
    pub claim_token_hash: Option<String>,
    /// When the registration window ends.
    pub claim_expires_at: Option<DateTime<Utc>>,
    /// The current claim attempt's identifier, `cla_` and 32 hex digits.
    pub attempt_id: Option<String>,
    /// Digest of the current claim attempt token.
    pub attempt_token_hash: Option<String>,
    /// Digest of the current user code.
    pub user_code_hash: Option<String>,
    /// When the current user code stops working.
    pub user_code_expires_at: Option<DateTime<Utc>>,
    /// How many wrong user codes the current attempt received.
    pub user_code_failures: u32,
    /// The whole second of the last accepted claim poll.
    pub last_polled_at: Option<DateTime<Utc>>,
    /// Digest of the provider's issuer and subject, for an ID-JAG
    /// registration, while it is not revoked. Unique: one registration per
    /// delegation.
    pub delegation_key: Option<String>,
    /// The provider's issuer, for an ID-JAG registration.
    pub issuer: Option<String>,
    /// The provider's subject, for an ID-JAG registration.
    pub subject: Option<String>,
    /// The scopes the registration holds once bound to a user.
    pub scopes: Vec<String>,
    /// The version identity assertions must carry. A claim moves it on, so
    /// an assertion signed before the claim stops working.
    pub assertion_version: u32,
    /// The bound user's authentication epoch when the registration was
    /// bound. Once the user's epoch moves on (a password change, signing
    /// out everywhere) the registration is ended, as sessions are.
    pub auth_epoch: Option<u64>,
    /// The client address the registration came from, for the audit trail
    /// of what happens to it later without a request, such as expiry.
    pub client_address: Option<String>,
    /// The authorization server grant the registration's tokens are minted
    /// under: at most one, reused by every exchange while it fits.
    pub grant_id: Option<String>,
    /// When the registration was created.
    pub created_at: DateTime<Utc>,
    /// When a user claimed it.
    pub claimed_at: Option<DateTime<Utc>>,
    /// When it was revoked or expired.
    pub revoked_at: Option<DateTime<Utc>>,
}

/// A new claim attempt: digests of its token and code, never the values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewAttempt {
    /// The attempt identifier.
    pub attempt_id: String,
    /// Digest of the attempt token.
    pub attempt_token_hash: String,
    /// Digest of the user code.
    pub user_code_hash: String,
    /// When the user code stops working.
    pub user_code_expires_at: DateTime<Utc>,
}

/// Storage for agent auth.
#[async_trait]
pub trait AgentAuthStore: Send + Sync {
    /// Insert a registration. A second registration with the same
    /// delegation key answers [`crate::Error::Conflict`].
    async fn create_registration(&self, record: RegistrationRecord) -> Result<()>;
    /// Find a registration by its identifier.
    async fn find_registration(&self, id: &str) -> Result<Option<RegistrationRecord>>;
    /// Find a registration by the digest of its claim token.
    async fn find_by_claim_token(&self, token_hash: &str) -> Result<Option<RegistrationRecord>>;
    /// Find a registration by the digest of its current attempt token.
    async fn find_by_attempt_token(&self, token_hash: &str) -> Result<Option<RegistrationRecord>>;
    /// Find the live registration of a delegation.
    async fn find_by_delegation(&self, delegation_key: &str) -> Result<Option<RegistrationRecord>>;
    /// Start a new claim attempt, replacing the current one, and bind the
    /// claim to `claim_email`: only while the registration still holds the
    /// claim token `claim_token_hash`, is unclaimed or pending, and its
    /// window is open at `now`. Returns whether this call won.
    async fn start_attempt(
        &self,
        id: &str,
        claim_token_hash: &str,
        claim_email: &str,
        attempt: NewAttempt,
        now: DateTime<Utc>,
    ) -> Result<bool>;
    /// Give a pending (or expired) ID-JAG step-up a new claim token, window
    /// and attempt, for a repeated presentation of the ID-JAG. Returns
    /// whether the registration was still waiting for its claim.
    async fn restart_step_up(
        &self,
        id: &str,
        claim_token_hash: &str,
        claim_expires_at: DateTime<Utc>,
        attempt: NewAttempt,
    ) -> Result<bool>;
    /// Count one wrong user code against the attempt, only while it has
    /// fewer than `max_failures`. Returns whether the count moved.
    async fn record_code_failure(
        &self,
        id: &str,
        attempt_token_hash: &str,
        max_failures: u32,
    ) -> Result<bool>;
    /// Mark the registration claimed by `user_id` at the user's
    /// `auth_epoch`, spend the attempt and move the assertion version on:
    /// only while the attempt token, the user code digest and the failure
    /// count still allow it and both the code and the window are open at
    /// `now`. Returns whether this call won.
    #[allow(clippy::too_many_arguments)]
    async fn confirm_claim(
        &self,
        id: &str,
        attempt_token_hash: &str,
        user_code_hash: &str,
        user_id: &str,
        auth_epoch: Option<u64>,
        max_failures: u32,
        now: DateTime<Utc>,
    ) -> Result<bool>;
    /// Record a claim poll in the whole second `slot`, only when the last
    /// one was at or before `earliest_previous`. Returns `false` for a poll
    /// that came too soon.
    async fn record_poll(
        &self,
        id: &str,
        slot: DateTime<Utc>,
        earliest_previous: DateTime<Utc>,
    ) -> Result<bool>;
    /// Spend the claim token of a claimed registration. Returns whether
    /// this call spent it.
    async fn spend_claim_token(&self, id: &str, claim_token_hash: &str) -> Result<bool>;
    /// Revoke a registration and forget its delegation and one-time
    /// tokens. Returns whether it was live.
    async fn revoke_registration(&self, id: &str, at: DateTime<Utc>) -> Result<bool>;
    /// Mark an unclaimed or pending registration whose window passed
    /// before `now` expired. Returns whether this call did.
    async fn expire_registration(&self, id: &str, now: DateTime<Utc>) -> Result<bool>;
    /// Up to `limit` unclaimed or pending registrations whose window passed
    /// before `now`.
    async fn expirable_registrations(&self, now: DateTime<Utc>, limit: u64) -> Result<Vec<String>>;
    /// Up to `limit` registrations that are not revoked.
    async fn live_registrations(&self, limit: u64) -> Result<Vec<String>>;
    /// The registrations bound to a user that are claimed or active.
    async fn user_registrations(&self, user_id: &str) -> Result<Vec<RegistrationRecord>>;
    /// Name `new_grant` as the registration's one grant, only while it
    /// still names `expected` (none when `None`) and still has `status` and
    /// `assertion_version`. Written before the grant is issued. Returns
    /// whether this call won.
    async fn reserve_grant(
        &self,
        id: &str,
        expected: Option<&str>,
        new_grant: &str,
        status: &str,
        assertion_version: u32,
    ) -> Result<bool>;
    /// Record a provider token's identifier until `expires_at`. Returns
    /// `false` when the record already exists: the token is a replay.
    async fn record_replay(&self, key: &str, expires_at: DateTime<Utc>) -> Result<bool>;
    /// Delete replay records that expired before `now`. Returns how many
    /// went.
    async fn prune_replays(&self, now: DateTime<Utc>) -> Result<u64>;
}
