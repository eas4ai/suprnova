//! RFC 8628 device authorization
//! (`docs/specs/suprnova-magnetar/09-oauth-engine.md`'s "Device
//! authorization" section).
//!
//! Every device-authorization request is tracked by two
//! [`crate::storage::CeremonyStore`] rows:
//!
//! - The **canonical row**, keyed by `user_code` under
//!   [`crate::storage::DeviceStore`]'s hardcoded kind
//!   (`src/storage/device.rs`'s private `DEVICE_KIND`, `"device-authorization"`
//!   -- this module's own [`DEVICE_CEREMONY_KIND`] constant must stay in
//!   sync with it), so [`verify`](DeviceAuthorizationService::verify),
//!   [`approve`](DeviceAuthorizationService::approve), and
//!   [`deny`](DeviceAuthorizationService::deny) use `DeviceStore`'s
//!   peek/transition contract directly. States are `"pending"`,
//!   `"approved:{user_id}"` (the approver's id embedded directly in the
//!   free-form `next` state string -- no third row is needed to remember
//!   who approved), `"denied"`, and `"issued"`.
//! - The **poll row**, keyed by `device_code` under this module's own
//!   [`DEVICE_POLL_KIND`], carrying an immutable pointer payload
//!   (`{user_code}`) plus a `"{interval_secs}:{last_poll_epoch_millis}"`
//!   state string that [`poll`](DeviceAuthorizationService::poll)
//!   CAS-transitions on every call, giving durable RFC 8628 §3.5 interval
//!   escalation without a new storage trait.
//!
//! [`approve`](DeviceAuthorizationService::approve) is the only place this
//! module touches [`FactorGate`]: it completes sign-in for the approving
//! actor and only records the approval once the gate allows a session,
//! embedding that fact in the canonical row's state. `poll` never
//! re-invokes the gate -- the embedded user id in an `"approved:*"` state
//! is proof the gate already ran, so a 2FA-enrolled user's device flow
//! never stalls waiting for a challenge the polling device (a TV, a CLI)
//! has no way to answer.
//!
//! `approve`'s `FactorGate::complete_sign_in` call is not a query: on
//! `SessionAllowed` it has already minted and persisted a real session for
//! the *approving actor* (not the device -- the device's own session comes
//! later, from whatever the caller does with
//! [`DevicePollOutcome::Success`]'s principal). `approve` surfaces that
//! grant as [`DeviceApprovalOutcome::Approved`]'s `approver_session` rather
//! than discarding it, and on the losing side of an approve/deny CAS race
//! *after* the gate has already run, best-effort revokes the
//! now-orphaned row before returning `Conflict` -- best-effort because the
//! revoke and the CAS are not one transaction: a crash between them still
//! leaves a live-but-unbound session row that lives to its own TTL.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};

use super::authorization::{decrypt, encrypt, new_selector};
use crate::abuse::{AbuseLimiter, AbusePolicy, Permit};
use crate::auth::{
    AuthenticationContext, FactorGate, SignInDecision, SignInMethod, VerifiedPrincipal,
};
use crate::crypto::Encryptor;
use crate::sessions::{SessionMetadata, SessionQueries};
use crate::storage::{CeremonyStore, DeviceStore, NewCeremony, UserStore};
use crate::{Error, Result};

/// Must match `src/storage/device.rs`'s private `DEVICE_KIND` -- the
/// canonical row is created through raw [`CeremonyStore::create`] (which
/// `DeviceStore` does not expose) but read/transitioned through
/// `DeviceStore`'s peek/transition convenience methods, so the kind string
/// must line up exactly.
const DEVICE_CEREMONY_KIND: &str = "device-authorization";
/// Kind namespace for the device_code-keyed polling-rate row.
const DEVICE_POLL_KIND: &str = "device-authorization-poll";
const PENDING: &str = "pending";
const DENIED: &str = "denied";
const ISSUED: &str = "issued";
const APPROVED_PREFIX: &str = "approved:";

/// A device client registered to use the device-authorization grant.
#[derive(Clone, Debug)]
pub struct DeviceClient {
    /// The client identifier `issue_code` is called with.
    pub client_id: String,
    /// The name shown to the human on the verification page.
    pub display_name: String,
    /// Scopes this client may request. `issue_code` rejects any requested
    /// scope outside this set.
    pub allowed_scopes: Vec<String>,
}

/// Device clients keyed by [`DeviceClient::client_id`].
#[derive(Default)]
pub struct DeviceClientRegistry {
    clients: HashMap<String, DeviceClient>,
}

impl DeviceClientRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one device client.
    ///
    /// # Errors
    /// Returns [`Error::Conflict`] when `client.client_id` is already
    /// registered.
    pub fn register(&mut self, client: DeviceClient) -> Result<&mut Self> {
        if self.clients.contains_key(&client.client_id) {
            return Err(Error::Conflict {
                resource: "device client".to_owned(),
                message: format!(
                    "a device client is already registered under id '{}'",
                    client.client_id
                ),
            });
        }
        self.clients.insert(client.client_id.clone(), client);
        Ok(self)
    }

    /// Look up a registered device client.
    #[must_use]
    pub fn get(&self, client_id: &str) -> Option<&DeviceClient> {
        self.clients.get(client_id)
    }
}

/// The RFC 8628 device_code/user_code pair and polling instructions
/// returned from [`DeviceAuthorizationService::issue_code`].
#[derive(Clone, Debug)]
pub struct DeviceCodeResponse {
    /// The opaque code the polling device presents to `poll`.
    pub device_code: secrecy::SecretString,
    /// The short, human-typable code the user presents to `verify`.
    pub user_code: String,
    /// The requesting client's display name (spec 09's device-code
    /// response contents; ux.md J7.1).
    pub client_display_name: String,
    /// The granted (validated, non-empty-filtered) scope set.
    pub scopes: Vec<String>,
    /// The URI the human visits to approve or deny the request.
    pub verification_uri: String,
    /// A `verification_uri` with `user_code` already embedded, when the
    /// host wants to render one (e.g. as a QR code). This module renders
    /// none: it is the host's route layer's job to compose one.
    pub verification_uri_complete: Option<String>,
    /// Seconds until the device_code/user_code pair expires.
    pub expires_in: u64,
    /// Recommended seconds between polls.
    pub interval: u64,
}

/// The ceremony's current decision, surfaced by
/// [`DeviceAuthorizationService::verify`] so a human landing on an
/// already-decided code sees that fact instead of a dead-ended approve/deny
/// prompt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceCeremonyStatus {
    /// Awaiting a decision; approve/deny are both still possible.
    Pending,
    /// Approved; polling will redeem it shortly.
    Approved,
    /// Denied.
    Denied,
    /// Already redeemed by a poll.
    Issued,
}

/// What [`DeviceAuthorizationService::verify`] shows a human deciding
/// whether to approve a device.
#[derive(Clone, Debug)]
pub struct DeviceDisplay {
    /// The requesting client's display name.
    pub client_display_name: String,
    /// The requested scopes.
    pub scopes: Vec<String>,
    /// The ceremony's current decision.
    pub status: DeviceCeremonyStatus,
}

/// The outcome of [`DeviceAuthorizationService::approve`].
#[derive(Debug)]
pub enum DeviceApprovalOutcome {
    /// The actor passed the factor gate and the ceremony is now approved.
    /// `FactorGate::complete_sign_in` genuinely mints and persists a
    /// session for the approving actor as a side effect of gating (see
    /// this module's doc); this carries that grant back to the caller
    /// rather than silently discarding a live, persisted session row.
    Approved {
        /// The session [`FactorGate::complete_sign_in`] minted for the
        /// approving actor while gating this decision.
        approver_session: Box<crate::sessions::SessionGrant>,
    },
    /// The actor is second-factor enrolled; approval is not recorded until
    /// this challenge is completed and `approve` is called again.
    FactorRequired {
        /// Selector used to submit the second-factor proof.
        challenge_selector: String,
    },
}

/// The RFC 8628 §3.5 poll outcome.
#[derive(Debug)]
pub enum DevicePollOutcome {
    /// The user has not yet decided.
    AuthorizationPending,
    /// The client is polling faster than the (possibly escalated)
    /// interval. `interval` is the new interval, in seconds, the client
    /// must wait between polls -- RFC 8628 §3.5 requires the server
    /// communicate the escalated value, not just reject the poll.
    SlowDown {
        /// Seconds to wait before the next poll.
        interval: u64,
    },
    /// The user denied the request.
    AccessDenied,
    /// The device_code is unknown, expired, or was already redeemed.
    ExpiredToken,
    /// The user approved the request. Carries the bare, gate-cleared
    /// principal for the caller to establish a session for the *device*
    /// with -- `poll` itself never mints one (matching
    /// [`crate::oauth::identity::IdentityResolver`]'s identity/session
    /// split). This is distinct from [`DeviceApprovalOutcome::Approved`]'s
    /// `approver_session`, which is the *approving actor's own* session,
    /// minted by `approve`'s `FactorGate::complete_sign_in` call, not by
    /// `poll`.
    Success(Box<VerifiedPrincipal>),
}

/// Tunables for one [`DeviceAuthorizationService`].
#[derive(Clone, Debug)]
pub struct DeviceAuthorizationConfig {
    /// How long a device_code/user_code pair remains valid.
    pub code_ttl: StdDuration,
    /// The base recommended polling interval (RFC 8628 default: 5s).
    pub poll_interval: StdDuration,
    /// The URI a human visits to approve or deny a device.
    pub verification_uri: String,
    /// The abuse-limiter budget applied to every `poll` call, independent
    /// of the interval-escalation mechanism above (defense in depth against
    /// a client that ignores the recommended interval entirely).
    pub poll_abuse_policy: AbusePolicy,
}

impl Default for DeviceAuthorizationConfig {
    fn default() -> Self {
        Self {
            code_ttl: StdDuration::from_secs(1800),
            poll_interval: StdDuration::from_secs(5),
            verification_uri: "https://auth.example.invalid/device".to_owned(),
            poll_abuse_policy: AbusePolicy {
                max_requests: 120,
                window: StdDuration::from_secs(60),
            },
        }
    }
}

#[derive(Serialize, Deserialize)]
struct DevicePayload {
    client_id: String,
    scopes: Vec<String>,
    display_name: String,
}

#[derive(Serialize, Deserialize)]
struct PollPayload {
    user_code: String,
}

/// RFC 8628 device authorization: issue/verify/approve/deny/poll.
pub struct DeviceAuthorizationService {
    ceremonies: Arc<dyn CeremonyStore>,
    devices: Arc<dyn DeviceStore>,
    users: Arc<dyn UserStore>,
    gate: Arc<dyn FactorGate>,
    /// Used only for the best-effort cleanup described on
    /// [`DeviceAuthorizationService::approve`]: `FactorGate::complete_sign_in`
    /// mints a real, persisted session as a side effect of gating, and if
    /// this service then loses the approve/deny CAS race, it revokes that
    /// orphaned row rather than leaving it live to TTL.
    ///
    /// **MUST be backed by the same [`crate::sessions::SessionQueries`]
    /// implementation as `gate`'s own session issuer.** A JWT-backed
    /// session provider's `revoke_session` is contractually `Ok(false)` --
    /// a self-contained token has no per-session row to revoke
    /// (`src/sessions/jwt.rs`'s doc: "global invalidation goes through the
    /// epoch") -- so pairing an opaque-session `gate` with a JWT-backed
    /// `sessions` here (or the reverse) would silently no-op this cleanup
    /// every time: the call still returns `Ok`, so nothing here can detect
    /// the mismatch.
    sessions: Arc<dyn SessionQueries>,
    limiter: Arc<dyn AbuseLimiter>,
    encryptor: Arc<dyn Encryptor>,
    clients: Arc<DeviceClientRegistry>,
    config: DeviceAuthorizationConfig,
}

impl DeviceAuthorizationService {
    /// Construct a device-authorization service over existing Magnetar
    /// stores. `sessions` **must** be backed by the same session-store
    /// implementation `gate` mints sessions through (see the `sessions`
    /// field's doc) -- constructing this with a mismatched pair compiles
    /// and runs, but silently defeats `approve`'s orphan-session cleanup.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ceremonies: Arc<dyn CeremonyStore>,
        devices: Arc<dyn DeviceStore>,
        users: Arc<dyn UserStore>,
        gate: Arc<dyn FactorGate>,
        sessions: Arc<dyn SessionQueries>,
        limiter: Arc<dyn AbuseLimiter>,
        encryptor: Arc<dyn Encryptor>,
        clients: Arc<DeviceClientRegistry>,
        config: DeviceAuthorizationConfig,
    ) -> Self {
        Self {
            ceremonies,
            devices,
            users,
            gate,
            sessions,
            limiter,
            encryptor,
            clients,
            config,
        }
    }

    /// Mint a device_code/user_code pair for a registered client.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] for an unregistered `client_id`.
    /// Returns [`Error::InvalidInput`] when `scope` requests a scope
    /// outside [`DeviceClient::allowed_scopes`].
    pub async fn issue_code(&self, client_id: &str, scope: &str) -> Result<DeviceCodeResponse> {
        let client = self.clients.get(client_id).ok_or_else(|| Error::NotFound {
            resource: "device client".to_owned(),
            identifier: client_id.to_owned(),
        })?;
        let requested: Vec<String> = scope
            .split_whitespace()
            .map(str::to_owned)
            .filter(|scope| !scope.is_empty())
            .collect();
        for scope in &requested {
            if !client.allowed_scopes.iter().any(|allowed| allowed == scope) {
                return Err(Error::InvalidInput {
                    field: "scope".to_owned(),
                    message: format!(
                        "scope '{scope}' is not permitted for device client '{client_id}'"
                    ),
                });
            }
        }

        let device_code = new_selector("device");
        let now = Utc::now();
        let ttl =
            ChronoDuration::from_std(self.config.code_ttl).map_err(|error| Error::Internal {
                message: format!("device code ttl out of range: {error}"),
            })?;
        let expires_at = now + ttl;
        let interval = self.config.poll_interval.as_secs();

        let device_payload = DevicePayload {
            client_id: client.client_id.clone(),
            scopes: requested.clone(),
            display_name: client.display_name.clone(),
        };
        let device_ciphertext = encrypt(self.encryptor.as_ref(), &device_payload)?;

        // `user_code`'s ~38 bits of entropy against an unbounded active set
        // makes a collision astronomically rare but not impossible; the
        // selector's uniqueness is enforced by the host's storage index
        // (`src/schema/ceremony.rs`'s "the unique selector"). Pre-check via
        // `peek` (read-only, cheap) rather than retrying `create` itself --
        // a bounded number of regenerate-and-peek round trips absorbs a
        // real collision without turning a single storage outage into five
        // immediate `create` attempts against a backend that is already
        // failing; the one `create` call below propagates its error
        // untouched.
        const MAX_USER_CODE_ATTEMPTS: u32 = 5;
        let mut user_code = random_user_code();
        for _ in 1..MAX_USER_CODE_ATTEMPTS {
            if self.devices.peek_device(&user_code).await?.is_none() {
                break;
            }
            user_code = random_user_code();
        }
        self.ceremonies
            .create(NewCeremony {
                selector: user_code.clone(),
                kind: DEVICE_CEREMONY_KIND.to_owned(),
                state: PENDING.to_owned(),
                payload: device_ciphertext,
                expires_at,
            })
            .await?;

        let poll_payload = PollPayload {
            user_code: user_code.clone(),
        };
        let poll_ciphertext = encrypt(self.encryptor.as_ref(), &poll_payload)?;
        self.ceremonies
            .create(NewCeremony {
                selector: device_code.clone(),
                kind: DEVICE_POLL_KIND.to_owned(),
                state: format!("{interval}:0"),
                payload: poll_ciphertext,
                expires_at,
            })
            .await?;

        Ok(DeviceCodeResponse {
            device_code: secrecy::SecretString::from(device_code),
            user_code,
            client_display_name: client.display_name.clone(),
            scopes: requested,
            verification_uri: self.config.verification_uri.clone(),
            verification_uri_complete: None,
            expires_in: self.config.code_ttl.as_secs(),
            interval,
        })
    }

    /// Read (without mutating) what a human deciding on `user_code` should
    /// see.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] when `user_code` is unknown or expired.
    pub async fn verify(&self, user_code: &str) -> Result<DeviceDisplay> {
        let user_code = normalize_user_code(user_code);
        let record = self
            .devices
            .peek_device(&user_code)
            .await?
            .ok_or_else(device_not_found)?;
        let payload: DevicePayload = decrypt(self.encryptor.as_ref(), &record.payload)?;
        let status = match record.state.as_str() {
            PENDING => DeviceCeremonyStatus::Pending,
            DENIED => DeviceCeremonyStatus::Denied,
            ISSUED => DeviceCeremonyStatus::Issued,
            state if state.starts_with(APPROVED_PREFIX) => DeviceCeremonyStatus::Approved,
            _ => DeviceCeremonyStatus::Pending,
        };
        Ok(DeviceDisplay {
            client_display_name: payload.display_name,
            scopes: payload.scopes,
            status,
        })
    }

    /// Approve `user_code` on behalf of `actor_user_id`, gated by
    /// [`FactorGate`]. Only one caller wins a pending ceremony.
    ///
    /// # Errors
    /// Returns [`Error::InvalidInput`] for an empty `actor_user_id`.
    /// Returns [`Error::NotFound`] when `user_code`/`actor_user_id` are
    /// unknown. Returns [`Error::Conflict`] when the ceremony was already
    /// decided (by this call or a concurrent one).
    pub async fn approve(
        &self,
        user_code: &str,
        actor_user_id: &str,
    ) -> Result<DeviceApprovalOutcome> {
        if actor_user_id.is_empty() {
            return Err(invalid("actor_user_id", "must not be empty"));
        }
        let user_code = normalize_user_code(user_code);
        let record = self
            .devices
            .peek_device(&user_code)
            .await?
            .ok_or_else(device_not_found)?;
        if record.state != PENDING {
            return Err(already_decided());
        }
        let user = self
            .users
            .find_by_id(actor_user_id)
            .await?
            .ok_or_else(|| Error::NotFound {
                resource: "user".to_owned(),
                identifier: actor_user_id.to_owned(),
            })?;
        let context =
            AuthenticationContext::new(SessionMetadata::default(), user.auth_epoch, Utc::now());
        let principal = VerifiedPrincipal::new(
            actor_user_id.to_owned(),
            SignInMethod::DeviceApproval,
            context.clone(),
        )?;

        match self.gate.complete_sign_in(principal, context).await? {
            SignInDecision::SessionAllowed(grant) => {
                let next = format!("{APPROVED_PREFIX}{actor_user_id}");
                if !self
                    .devices
                    .transition_device(&user_code, PENDING, &next)
                    .await?
                {
                    // Lost the race after the gate already minted and
                    // persisted a real session for this actor. Best-effort
                    // clean it up rather than leave it live to TTL -- see
                    // this module's doc for the crash-window residual this
                    // cannot close (the revoke and the lost CAS are not one
                    // transaction).
                    let _ = self.sessions.revoke_session(grant.session_id()).await;
                    return Err(already_decided());
                }
                Ok(DeviceApprovalOutcome::Approved {
                    approver_session: Box::new(grant),
                })
            }
            SignInDecision::FactorRequired { challenge_selector } => {
                Ok(DeviceApprovalOutcome::FactorRequired { challenge_selector })
            }
        }
    }

    /// Deny `user_code` on behalf of `actor_user_id`. Only one caller wins
    /// a pending ceremony.
    ///
    /// # Errors
    /// Returns [`Error::InvalidInput`] for an empty `actor_user_id`.
    /// Returns [`Error::NotFound`] when `user_code` is unknown or expired.
    /// Returns [`Error::Conflict`] when the ceremony was already decided.
    pub async fn deny(&self, user_code: &str, actor_user_id: &str) -> Result<()> {
        if actor_user_id.is_empty() {
            return Err(invalid("actor_user_id", "must not be empty"));
        }
        let user_code = normalize_user_code(user_code);
        let record = self
            .devices
            .peek_device(&user_code)
            .await?
            .ok_or_else(device_not_found)?;
        if record.state != PENDING {
            return Err(already_decided());
        }
        if !self
            .devices
            .transition_device(&user_code, PENDING, DENIED)
            .await?
        {
            return Err(already_decided());
        }
        Ok(())
    }

    /// Poll for a decision on `device_code`.
    ///
    /// # Errors
    /// Propagates [`AbuseLimiter::acquire`]/store/decrypt failures --
    /// polling fails closed rather than treating a limiter backend failure
    /// as [`Permit::Allowed`].
    pub async fn poll(&self, device_code: &str) -> Result<DevicePollOutcome> {
        // Consulted unconditionally, before any lookup, so that guessing
        // device_codes is rate-limited exactly like polling a real one --
        // an unconditional early return on an unknown code would let a
        // caller enumerate codes at whatever rate it likes.
        let key = format!("device-poll:{device_code}");
        let permit = self
            .limiter
            .acquire(&key, self.config.poll_abuse_policy)
            .await?;

        let Some(poll_record) = self.ceremonies.peek(device_code, DEVICE_POLL_KIND).await? else {
            return Ok(match permit {
                Permit::Rejected { .. } => DevicePollOutcome::SlowDown {
                    interval: self.config.poll_interval.as_secs(),
                },
                Permit::Allowed { .. } => DevicePollOutcome::ExpiredToken,
            });
        };
        let poll_payload: PollPayload = decrypt(self.encryptor.as_ref(), &poll_record.payload)?;
        let (interval, last_poll_millis) = parse_poll_state(&poll_record.state)?;

        if let Permit::Rejected { .. } = permit {
            return Ok(DevicePollOutcome::SlowDown { interval });
        }

        let Some(canonical) = self.devices.peek_device(&poll_payload.user_code).await? else {
            return Ok(DevicePollOutcome::ExpiredToken);
        };

        match canonical.state.as_str() {
            // RFC 8628 §3.5: slow_down is a variant of authorization_pending
            // -- the interval-escalation check applies only here. A denied,
            // issued, or freshly approved ceremony resolves immediately
            // regardless of how fast the client is polling; only an
            // undecided ceremony can ever be told to slow down.
            PENDING => {
                let now = Utc::now();
                let now_millis = now.timestamp_millis();
                let elapsed_secs = (now_millis.saturating_sub(last_poll_millis)) as f64 / 1000.0;
                if last_poll_millis > 0 && elapsed_secs < interval as f64 {
                    // Escalate, but never past the code's own remaining
                    // lifetime -- otherwise a client that ignores
                    // `slow_down` could drive its own interval beyond
                    // `expires_in`, making the flow unrecoverable even
                    // once it starts behaving.
                    let remaining_secs = (poll_record.expires_at - now).num_seconds().max(1) as u64;
                    let escalated = (interval + 5).min(remaining_secs);
                    let next_state = format!("{escalated}:{now_millis}");
                    // Under concurrent polls both readers may see the same
                    // state and both attempt this CAS; the loser's
                    // `last_poll_millis` update is silently dropped. This
                    // is advisory-only by design -- the abuse limiter
                    // above is the hard control, and terminal issuance is
                    // the single-winner CAS on the canonical row below.
                    let _ = self
                        .ceremonies
                        .transition(
                            device_code,
                            DEVICE_POLL_KIND,
                            &poll_record.state,
                            &next_state,
                        )
                        .await?;
                    return Ok(DevicePollOutcome::SlowDown {
                        interval: escalated,
                    });
                }
                let next_state = format!("{interval}:{now_millis}");
                // Advisory-only, same as above: a lost race here only
                // means one fewer poll's timestamp was recorded.
                let _ = self
                    .ceremonies
                    .transition(
                        device_code,
                        DEVICE_POLL_KIND,
                        &poll_record.state,
                        &next_state,
                    )
                    .await?;
                Ok(DevicePollOutcome::AuthorizationPending)
            }
            DENIED => Ok(DevicePollOutcome::AccessDenied),
            ISSUED => Ok(DevicePollOutcome::ExpiredToken),
            state if state.starts_with(APPROVED_PREFIX) => {
                let user_id = state[APPROVED_PREFIX.len()..].to_owned();
                if self
                    .devices
                    .transition_device(&poll_payload.user_code, state, ISSUED)
                    .await?
                {
                    let user =
                        self.users
                            .find_by_id(&user_id)
                            .await?
                            .ok_or_else(|| Error::NotFound {
                                resource: "user".to_owned(),
                                identifier: user_id.clone(),
                            })?;
                    let principal = VerifiedPrincipal::new(
                        user_id,
                        SignInMethod::DeviceApproval,
                        AuthenticationContext::new(
                            SessionMetadata::default(),
                            user.auth_epoch,
                            Utc::now(),
                        ),
                    )?;
                    Ok(DevicePollOutcome::Success(Box::new(principal)))
                } else {
                    // Lost the single-winner race: a concurrent poll already
                    // consumed this device_code.
                    Ok(DevicePollOutcome::ExpiredToken)
                }
            }
            _ => Ok(DevicePollOutcome::ExpiredToken),
        }
    }
}

fn parse_poll_state(state: &str) -> Result<(u64, i64)> {
    let (interval_str, last_poll_str) = state.split_once(':').ok_or_else(|| Error::Internal {
        message: format!("malformed device poll state: {state}"),
    })?;
    let interval: u64 = interval_str.parse().map_err(|_| Error::Internal {
        message: format!("malformed device poll interval: {state}"),
    })?;
    let last_poll: i64 = last_poll_str.parse().map_err(|_| Error::Internal {
        message: format!("malformed device poll timestamp: {state}"),
    })?;
    Ok((interval, last_poll))
}

/// A short, human-typable RFC 8628 §6.1 user code: 8 characters from an
/// alphabet excluding vowels and visually ambiguous characters (`0`/`O`,
/// `1`/`I`/`L`), grouped for readability.
fn random_user_code() -> String {
    const ALPHABET: &[u8] = b"BCDFGHJKMNPQRTVWXY23456789";
    let mut raw = [0_u8; 8];
    for slot in &mut raw {
        let index = (rand::random::<u32>() as usize) % ALPHABET.len();
        *slot = ALPHABET[index];
    }
    let text = String::from_utf8(raw.to_vec()).expect("alphabet is ascii");
    format!("{}-{}", &text[..4], &text[4..])
}

/// Normalize human-typed input for [`DeviceAuthorizationService::verify`]/
/// `approve`/`deny` to `random_user_code`'s canonical stored form: RFC 8628
/// §6.1 recommends the authorization server "SHOULD ... be insensitive to
/// transcription errors" -- case and the display hyphen are the two this
/// implementation's alphabet makes ambiguous, so both are normalized away
/// before the lookup. Anything other than 8 alphanumeric characters after
/// stripping is left ungrouped, which cannot match a real selector and so
/// correctly falls through to `NotFound`.
fn normalize_user_code(input: &str) -> String {
    let cleaned: String = input
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if cleaned.len() == 8 {
        format!("{}-{}", &cleaned[..4], &cleaned[4..])
    } else {
        cleaned
    }
}

fn invalid(field: &str, message: &str) -> Error {
    Error::InvalidInput {
        field: field.to_owned(),
        message: message.to_owned(),
    }
}

fn device_not_found() -> Error {
    Error::NotFound {
        resource: "device authorization".to_owned(),
        identifier: "user_code".to_owned(),
    }
}

fn already_decided() -> Error {
    Error::Conflict {
        resource: "device authorization".to_owned(),
        message: "this device authorization request was already decided".to_owned(),
    }
}
