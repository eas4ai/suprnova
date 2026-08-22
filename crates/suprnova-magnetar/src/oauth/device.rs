//! First-party device sign-in with human approval and single-use session delivery.
//!
//! A short user code is the canonical approval authority. The device code only
//! identifies a polling row that points at that user code. Approval always
//! passes through the shared factor gate. The resulting opaque session grant is
//! encrypted under the dedicated session-grant crypto purpose and stored in a
//! single-use ceremony row; polling consumes that row atomically, so at most one
//! caller can receive the bearer.

use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};

use super::authorization::{decrypt, encrypt, new_selector};
use crate::abuse::{AbuseLimiter, AbusePolicy, Permit};
use crate::auth::{
    AuthenticationContext, FactorGate, SignInDecision, SignInMethod, VerifiedPrincipal,
};
use crate::crypto::{CryptoPurpose, Encryptor};
use crate::sessions::{SessionGrant, SessionMetadata, SessionQueries};
use crate::storage::{CeremonyStore, CredentialActor, DeviceStore, NewCeremony, UserStore};
use crate::{Error, Result};

const DEVICE_CEREMONY_KIND: &str = "device-authorization";
const DEVICE_POLL_KIND: &str = "device-authorization-poll";
const DEVICE_GRANT_KIND: &str = "device-authorization-grant";
const DEVICE_CONTINUATION_KIND: &str = "device-authorization-continuation";
const PENDING: &str = "pending";
const AVAILABLE: &str = "available";
const APPROVING_PREFIX: &str = "approving:";
const DENIED: &str = "denied";
const ISSUED: &str = "issued";
const APPROVED_PREFIX: &str = "approved:";

/// The device-code/user-code pair and polling instructions.
#[derive(Clone, Debug)]
pub struct DeviceCodeResponse {
    /// The opaque code the first-party device presents to [`DeviceAuthorizationService::poll`].
    pub device_code: secrecy::SecretString,
    /// The short, human-typable code presented on the approval device.
    pub user_code: String,
    /// The URI the human visits to approve or deny the request.
    pub verification_uri: String,
    /// A complete URI is optional because route composition belongs to the host.
    pub verification_uri_complete: Option<String>,
    /// Seconds until the pair expires.
    pub expires_in: u64,
    /// Recommended seconds between polls.
    pub interval: u64,
}

/// The ceremony's current decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceCeremonyStatus {
    /// Awaiting a decision.
    Pending,
    /// Approved and waiting for the device to redeem the session.
    Approved,
    /// Denied.
    Denied,
    /// Already redeemed.
    Issued,
}

/// First-party state shown to the human deciding a user code.
#[derive(Clone, Debug)]
pub struct DeviceDisplay {
    /// The ceremony's current decision.
    pub status: DeviceCeremonyStatus,
    /// Absolute expiry of the user code.
    pub expires_at: DateTime<Utc>,
}

/// Outcome of beginning an approval.
#[derive(Debug)]
pub enum DeviceApprovalOutcome {
    /// The factor policy was satisfied and an encrypted device session is ready.
    Approved,
    /// A second factor must be completed through [`DeviceAuthorizationService::complete_approval`].
    FactorRequired {
        /// Selector for the factor proof.
        challenge_selector: String,
    },
}

/// Poll result for a first-party device.
#[derive(Debug)]
pub enum DevicePollOutcome {
    /// The human has not decided yet.
    AuthorizationPending,
    /// The device polled too quickly.
    SlowDown {
        /// New minimum polling interval in seconds.
        interval: u64,
    },
    /// The human denied the request.
    AccessDenied,
    /// The code is unknown, expired, or already redeemed.
    ExpiredToken,
    /// The one-time, already-persisted Magnetar session grant.
    Success(Box<SessionGrant>),
}

/// Tunables for one device sign-in service.
#[derive(Clone, Debug)]
pub struct DeviceAuthorizationConfig {
    /// How long a device/user code pair remains valid.
    pub code_ttl: StdDuration,
    /// Base polling interval.
    pub poll_interval: StdDuration,
    /// Human-facing approval URI.
    pub verification_uri: String,
    /// Defense-in-depth limiter policy applied to every poll.
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
    version: u8,
}

#[derive(Serialize, Deserialize)]
struct PollPayload {
    user_code: String,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
struct CredentialActorSnapshot {
    user_id: String,
    issuance_epoch: u64,
    opaque_session_id: Option<String>,
    expires_at: Option<DateTime<Utc>>,
}

impl CredentialActorSnapshot {
    fn capture(actor: &CredentialActor) -> Self {
        Self {
            user_id: actor.user_id().to_owned(),
            issuance_epoch: actor.issuance_epoch(),
            opaque_session_id: actor.opaque_session_id().map(str::to_owned),
            expires_at: actor.expires_at(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct ApprovalContinuation {
    user_code: String,
    actor: CredentialActorSnapshot,
}

/// First-party device sign-in: issue, inspect, approve/deny, and poll.
pub struct DeviceAuthorizationService {
    ceremonies: Arc<dyn CeremonyStore>,
    devices: Arc<dyn DeviceStore>,
    users: Arc<dyn UserStore>,
    gate: Arc<dyn FactorGate>,
    sessions: Arc<dyn SessionQueries>,
    limiter: Arc<dyn AbuseLimiter>,
    encryptor: Arc<dyn Encryptor>,
    config: DeviceAuthorizationConfig,
}

impl DeviceAuthorizationService {
    /// Construct the service over the host's shared stores, gate, and crypto boundary.
    ///
    /// `sessions` must address the same opaque session store used by `gate` so a
    /// session minted before a lost storage race can be revoked.
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
            config,
        }
    }

    /// Mint a first-party device/user code pair.
    pub async fn issue_code(&self) -> Result<DeviceCodeResponse> {
        let device_code = new_selector("device");
        let now = Utc::now();
        let ttl =
            ChronoDuration::from_std(self.config.code_ttl).map_err(|error| Error::Internal {
                message: format!("device code ttl out of range: {error}"),
            })?;
        let expires_at = now + ttl;
        let interval = self.config.poll_interval.as_secs();
        let device_ciphertext = encrypt(self.encryptor.as_ref(), &DevicePayload { version: 1 })?;

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

        let poll_ciphertext = encrypt(
            self.encryptor.as_ref(),
            &PollPayload {
                user_code: user_code.clone(),
            },
        )?;
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
            verification_uri: self.config.verification_uri.clone(),
            verification_uri_complete: None,
            expires_in: self.config.code_ttl.as_secs(),
            interval,
        })
    }

    /// Inspect a user code without mutating it.
    pub async fn verify(&self, user_code: &str) -> Result<DeviceDisplay> {
        let user_code = normalize_user_code(user_code);
        let record = self
            .devices
            .peek_device(&user_code)
            .await?
            .ok_or_else(device_not_found)?;
        let _: DevicePayload = decrypt(self.encryptor.as_ref(), &record.payload)?;
        let status = match record.state.as_str() {
            state if state == PENDING || state.starts_with(APPROVING_PREFIX) => {
                DeviceCeremonyStatus::Pending
            }
            DENIED => DeviceCeremonyStatus::Denied,
            ISSUED => DeviceCeremonyStatus::Issued,
            state if state.starts_with(APPROVED_PREFIX) => DeviceCeremonyStatus::Approved,
            _ => DeviceCeremonyStatus::Pending,
        };
        Ok(DeviceDisplay {
            status,
            expires_at: record.expires_at,
        })
    }

    /// Begin approval using a trusted request-derived actor and device metadata.
    pub async fn approve(
        &self,
        user_code: &str,
        actor: &CredentialActor,
        metadata: SessionMetadata,
    ) -> Result<DeviceApprovalOutcome> {
        self.validate_actor(actor).await?;
        let user_code = normalize_user_code(user_code);
        let record = self
            .devices
            .peek_device(&user_code)
            .await?
            .ok_or_else(device_not_found)?;
        if record.state != PENDING {
            return Err(already_decided());
        }

        let context = AuthenticationContext::new(metadata, actor.issuance_epoch(), Utc::now());
        let principal = VerifiedPrincipal::new(
            actor.user_id().to_owned(),
            SignInMethod::DeviceApproval,
            context.clone(),
        )?;
        match self.gate.complete_sign_in(principal, context).await? {
            SignInDecision::SessionAllowed(grant) => {
                self.persist_approval(&user_code, record.expires_at, grant, PENDING)
                    .await?;
                Ok(DeviceApprovalOutcome::Approved)
            }
            SignInDecision::FactorRequired { challenge_selector } => {
                let continuation_selector = continuation_selector(&challenge_selector);
                let payload = ApprovalContinuation {
                    user_code,
                    actor: CredentialActorSnapshot::capture(actor),
                };
                let ciphertext = encrypt(self.encryptor.as_ref(), &payload)?;
                if let Err(error) = self
                    .ceremonies
                    .create(NewCeremony {
                        selector: continuation_selector,
                        kind: DEVICE_CONTINUATION_KIND.to_owned(),
                        state: PENDING.to_owned(),
                        payload: ciphertext,
                        expires_at: record.expires_at,
                    })
                    .await
                {
                    let _ = self
                        .ceremonies
                        .consume(&challenge_selector, crate::auth::TWO_FACTOR_CHALLENGE_KIND)
                        .await;
                    return Err(error);
                }
                Ok(DeviceApprovalOutcome::FactorRequired { challenge_selector })
            }
        }
    }

    /// Complete the factor challenge bound to the original actor and user code.
    pub async fn complete_approval(
        &self,
        challenge_selector: &str,
        code: &str,
        actor: &CredentialActor,
    ) -> Result<()> {
        self.validate_actor(actor).await?;
        let continuation_selector = continuation_selector(challenge_selector);
        let continuation_record = self
            .ceremonies
            .peek(&continuation_selector, DEVICE_CONTINUATION_KIND)
            .await?
            .ok_or_else(approval_continuation_not_found)?;
        let continuation: ApprovalContinuation =
            decrypt(self.encryptor.as_ref(), &continuation_record.payload)?;
        if continuation.actor != CredentialActorSnapshot::capture(actor) {
            return Err(stale_actor());
        }

        let canonical = self
            .devices
            .peek_device(&continuation.user_code)
            .await?
            .ok_or_else(device_not_found)?;
        if canonical.state != PENDING {
            return Err(already_decided());
        }
        let reservation = format!("{APPROVING_PREFIX}{}", new_selector("device-claim"));
        if !self
            .devices
            .transition_device(&continuation.user_code, PENDING, &reservation)
            .await?
        {
            return Err(already_decided());
        }

        let grant = match self.gate.complete_challenge(challenge_selector, code).await {
            Ok(grant) => grant,
            Err(error) => {
                let _ = self
                    .devices
                    .transition_device(&continuation.user_code, &reservation, PENDING)
                    .await;
                return Err(error);
            }
        };
        let session_id = grant.session_id().to_owned();
        let consumed = match self
            .ceremonies
            .consume(&continuation_selector, DEVICE_CONTINUATION_KIND)
            .await
        {
            Ok(consumed) => consumed,
            Err(error) => {
                let _ = self.sessions.revoke_session(&session_id).await;
                let _ = self
                    .devices
                    .transition_device(&continuation.user_code, &reservation, PENDING)
                    .await;
                return Err(error);
            }
        };
        if consumed.is_none() {
            let _ = self.sessions.revoke_session(&session_id).await;
            let _ = self
                .devices
                .transition_device(&continuation.user_code, &reservation, PENDING)
                .await;
            return Err(approval_continuation_not_found());
        }

        let result = self
            .persist_approval(
                &continuation.user_code,
                canonical.expires_at,
                grant,
                &reservation,
            )
            .await;
        if result.is_err() {
            let _ = self
                .devices
                .transition_device(&continuation.user_code, &reservation, PENDING)
                .await;
        }
        result
    }

    /// Deny a pending user code on behalf of a trusted request-derived actor.
    pub async fn deny(&self, user_code: &str, actor: &CredentialActor) -> Result<()> {
        self.validate_actor(actor).await?;
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

    /// Poll for a decision and atomically redeem the encrypted device session.
    pub async fn poll(&self, device_code: &str) -> Result<DevicePollOutcome> {
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
            state if state == PENDING || state.starts_with(APPROVING_PREFIX) => {
                let now = Utc::now();
                let now_millis = now.timestamp_millis();
                let elapsed_secs = (now_millis.saturating_sub(last_poll_millis)) as f64 / 1000.0;
                if last_poll_millis > 0 && elapsed_secs < interval as f64 {
                    let remaining_secs = (poll_record.expires_at - now).num_seconds().max(1) as u64;
                    let escalated = (interval + 5).min(remaining_secs);
                    let next_state = format!("{escalated}:{now_millis}");
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
                let grant_selector = &state[APPROVED_PREFIX.len()..];
                let Some(grant_record) = self
                    .ceremonies
                    .consume(grant_selector, DEVICE_GRANT_KIND)
                    .await?
                else {
                    let _ = self
                        .devices
                        .transition_device(&poll_payload.user_code, state, ISSUED)
                        .await?;
                    return Ok(DevicePollOutcome::ExpiredToken);
                };
                let grant = self.decrypt_grant(&grant_record.payload)?;
                let _ = self
                    .devices
                    .transition_device(&poll_payload.user_code, state, ISSUED)
                    .await?;
                Ok(DevicePollOutcome::Success(Box::new(grant)))
            }
            _ => Ok(DevicePollOutcome::ExpiredToken),
        }
    }

    async fn validate_actor(&self, actor: &CredentialActor) -> Result<()> {
        if actor.user_id().is_empty()
            || actor
                .expires_at()
                .is_some_and(|expires_at| expires_at <= Utc::now())
        {
            return Err(stale_actor());
        }
        let user = self
            .users
            .find_by_id(actor.user_id())
            .await?
            .ok_or_else(stale_actor)?;
        if user.auth_epoch != actor.issuance_epoch() {
            return Err(stale_actor());
        }
        Ok(())
    }

    async fn persist_approval(
        &self,
        user_code: &str,
        expires_at: DateTime<Utc>,
        grant: SessionGrant,
        expected_state: &str,
    ) -> Result<()> {
        let session_id = grant.session_id().to_owned();
        let grant_selector = new_selector("device-grant");
        let payload = match self.encrypt_grant(grant) {
            Ok(payload) => payload,
            Err(error) => {
                let _ = self.sessions.revoke_session(&session_id).await;
                return Err(error);
            }
        };
        if let Err(error) = self
            .ceremonies
            .create(NewCeremony {
                selector: grant_selector.clone(),
                kind: DEVICE_GRANT_KIND.to_owned(),
                state: AVAILABLE.to_owned(),
                payload,
                expires_at,
            })
            .await
        {
            let _ = self.sessions.revoke_session(&session_id).await;
            return Err(error);
        }

        let next = format!("{APPROVED_PREFIX}{grant_selector}");
        match self
            .devices
            .transition_device(user_code, expected_state, &next)
            .await
        {
            Ok(true) => Ok(()),
            Ok(false) => {
                let _ = self
                    .ceremonies
                    .consume(&grant_selector, DEVICE_GRANT_KIND)
                    .await;
                let _ = self.sessions.revoke_session(&session_id).await;
                Err(already_decided())
            }
            Err(error) => {
                let _ = self
                    .ceremonies
                    .consume(&grant_selector, DEVICE_GRANT_KIND)
                    .await;
                let _ = self.sessions.revoke_session(&session_id).await;
                Err(error)
            }
        }
    }

    fn encrypt_grant(&self, grant: SessionGrant) -> Result<Vec<u8>> {
        let plaintext =
            serde_json::to_vec(&grant.into_snapshot()).map_err(|error| Error::Internal {
                message: format!("device session serialization failed: {error}"),
            })?;
        self.encryptor
            .encrypt(CryptoPurpose::SessionGrant, &plaintext)
    }

    fn decrypt_grant(&self, ciphertext: &[u8]) -> Result<SessionGrant> {
        let plaintext = self
            .encryptor
            .decrypt(CryptoPurpose::SessionGrant, ciphertext)?;
        let snapshot = serde_json::from_slice(&plaintext).map_err(|error| Error::InvalidInput {
            field: "device_code".to_owned(),
            message: format!("invalid encrypted device session: {error}"),
        })?;
        SessionGrant::from_snapshot(snapshot)
    }
}

fn continuation_selector(challenge_selector: &str) -> String {
    format!("device-approval-{challenge_selector}")
}

fn parse_poll_state(state: &str) -> Result<(u64, i64)> {
    let (interval_str, last_poll_str) = state.split_once(':').ok_or_else(|| Error::Internal {
        message: "invalid device poll state".to_owned(),
    })?;
    let interval = interval_str.parse().map_err(|_| Error::Internal {
        message: "invalid device poll interval".to_owned(),
    })?;
    let last_poll = last_poll_str.parse().map_err(|_| Error::Internal {
        message: "invalid device poll timestamp".to_owned(),
    })?;
    Ok((interval, last_poll))
}

fn random_user_code() -> String {
    const ALPHABET: &[u8] = b"23456789BCDFGHJKMNPQRSTVWXYZ";
    let mut bytes = [0_u8; 8];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut bytes);
    let rendered: String = bytes
        .iter()
        .map(|byte| ALPHABET[usize::from(*byte) % ALPHABET.len()] as char)
        .collect();
    format!("{}-{}", &rendered[..4], &rendered[4..])
}

fn normalize_user_code(input: &str) -> String {
    let compact: String = input
        .chars()
        .filter(|character| *character != '-')
        .flat_map(char::to_uppercase)
        .collect();
    if compact.len() == 8
        && compact
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
    {
        format!("{}-{}", &compact[..4], &compact[4..])
    } else {
        compact
    }
}

fn device_not_found() -> Error {
    Error::NotFound {
        resource: "device authorization".to_owned(),
        identifier: "unknown or expired code".to_owned(),
    }
}

fn approval_continuation_not_found() -> Error {
    Error::NotFound {
        resource: "device approval".to_owned(),
        identifier: "unknown, expired, or already completed challenge".to_owned(),
    }
}

fn stale_actor() -> Error {
    Error::NotFound {
        resource: "credential actor".to_owned(),
        identifier: "expired or revoked".to_owned(),
    }
}

fn already_decided() -> Error {
    Error::Conflict {
        resource: "device authorization".to_owned(),
        message: "device authorization was already decided".to_owned(),
    }
}
