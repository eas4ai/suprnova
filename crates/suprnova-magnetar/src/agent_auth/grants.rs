//! The two grants agent auth adds to the token endpoint, and the listener
//! that records revocations of agent tokens.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;

use super::jwt::verify_assertion;
use super::store::{
    RegistrationRecord, STATUS_CLAIMED, STATUS_EXPIRED, STATUS_REVOKED, STATUS_UNCLAIMED,
};
use super::{AGENT_GRANT_KIND, AgentUser, CLAIM_GRANT, Core, JWT_BEARER_GRANT, digest, timestamp};
use crate::abuse::Permit;
use crate::authorization_server::{
    GrantHandler, RevocationListener, RevokedToken, TokenError, TokenIssuer, TokenRequest,
    TokenResponse,
};
use crate::plugin::LifecycleEventKind;

/// The longest assertion or claim token the grants read.
const MAX_PARAMETER_LENGTH: usize = 16 * 1024;

fn storage(error: &crate::Error) -> TokenError {
    tracing::error!(error = %error, "agent auth storage failed");
    TokenError::server_error()
}

/// The JWT bearer grant (RFC 7523): an agent exchanges its identity
/// assertion for an access token.
pub(crate) struct JwtBearerGrant {
    core: Arc<Core>,
}

impl JwtBearerGrant {
    pub(crate) fn new(core: Arc<Core>) -> Self {
        Self { core }
    }
}

#[async_trait]
impl GrantHandler for JwtBearerGrant {
    fn grant_type(&self) -> &str {
        JWT_BEARER_GRANT
    }

    async fn exchange(
        &self,
        request: &TokenRequest,
        issuer: &TokenIssuer<'_>,
    ) -> Result<TokenResponse, TokenError> {
        let core = &self.core;
        let assertion = request
            .param("assertion")
            .ok_or_else(|| TokenError::invalid_request("assertion is required"))?;
        if assertion.len() > MAX_PARAMETER_LENGTH {
            return Err(TokenError::invalid_grant("the assertion is invalid"));
        }
        let now = Utc::now();
        // A previous key verifies only until it retires.
        let keys = std::iter::once(&core.config.signing_key).chain(
            core.config
                .previous_signing_keys
                .iter()
                .filter(|previous| {
                    previous
                        .retires_at(core.config.assertion_lifetime)
                        .is_some_and(|retirement| retirement > now)
                })
                .map(|previous| &previous.key),
        );
        let claims = verify_assertion(assertion, keys, &core.issuer, now)
            .map_err(TokenError::invalid_grant)?;
        let record = core
            .store
            .find_registration(&claims.sub)
            .await
            .map_err(|error| storage(&error))?
            .ok_or_else(|| TokenError::invalid_grant("the registration does not exist"))?;
        if record.cleanup_pending {
            // A revocation or expiry whose grant revocation failed is
            // finished here, before the refusal below, so presenting the
            // assertion again retries it.
            core.finish_pending_cleanup(issuer, &record, request.client_address())
                .await;
        }
        if record.assertion_version != claims.ver {
            return Err(TokenError::invalid_grant(
                "the assertion was superseded; use the one the claim returned",
            ));
        }
        let unclaimed = record.status == STATUS_UNCLAIMED;
        if !(unclaimed || Core::is_bound(&record)) {
            return Err(TokenError::invalid_grant(
                "the registration is not active: pending, expired or revoked",
            ));
        }
        if unclaimed && record.claim_expires_at.is_none_or(|window| window <= now) {
            return Err(TokenError::invalid_grant("the registration expired"));
        }
        // The request is checked in full before any grant is touched, so a
        // wrong scope or resource revokes nothing.
        let entitlement = if unclaimed {
            core.config.pre_claim_scopes.clone()
        } else {
            core.bound_scopes(&record)
        };
        let scopes = match request.param("scope") {
            None => entitlement.clone(),
            Some(scope) => {
                let requested: Vec<&str> = scope.split_whitespace().collect();
                if let Some(outside) = requested
                    .iter()
                    .find(|name| !entitlement.iter().any(|allowed| allowed == *name))
                {
                    return Err(TokenError::invalid_scope(format!(
                        "scope {outside:?} is not held by this registration"
                    )));
                }
                entitlement
                    .iter()
                    .filter(|name| requested.contains(&name.as_str()))
                    .cloned()
                    .collect()
            }
        };
        let Some(resource) = core.accepted_resource(
            request
                .param("resource")
                .unwrap_or(core.default_resource.as_str()),
        ) else {
            return Err(TokenError::invalid_target(
                "the resource is not served here",
            ));
        };
        let user = bound_user(core, &record).await?;
        if let Some(user) = &user
            && (Core::epoch_moved(&record, user)
                || (user.auth_epoch.is_some() && claims.auth_epoch != user.auth_epoch))
        {
            return Err(end_for_epoch(core, issuer, &record, request.client_address()).await);
        }
        // Each exchange mints a token, so each one spends the
        // registration's own budget, after the assertion and the
        // registration checked out.
        let key = crate::plugins::abuse_key("agent-auth.exchange", &record.id);
        match core
            .limiter
            .acquire(&key, core.config.limits.exchange)
            .await
        {
            Ok(Permit::Allowed { .. }) => {}
            Ok(Permit::Rejected { .. }) => {
                return Err(TokenError::new(
                    429,
                    "rate_limited",
                    Some("too many exchanges for this registration; back off and retry".to_owned()),
                ));
            }
            Err(error) => {
                tracing::error!(error = %error, "abuse limiter unavailable; failing closed");
                return Err(TokenError::new(503, "temporarily_unavailable", None));
            }
        }
        let (response, _) = core
            .issue_for_registration(
                issuer,
                &record,
                user.as_ref(),
                entitlement,
                scopes,
                resource,
            )
            .await?;
        core.emit(
            LifecycleEventKind::AgentTokenIssued,
            user.as_ref().map(|user| user.user_id.as_str()),
            &[
                ("registration_id", record.id.clone()),
                ("scope", response.scope.clone()),
                ("grant_type", JWT_BEARER_GRANT.to_owned()),
            ],
            request.client_address(),
        )
        .await;
        Ok(response)
    }
}

/// End a bound registration whose user's epoch moved on, and answer for
/// it. The answer is `invalid_grant` whether or not the revocation was
/// stored: the assertion must not work. A failed revocation is retried by
/// the next presentation: one that failed before the row was revoked finds
/// the epoch still moved, and one that failed at the grant finds the
/// clean-up pending.
async fn end_for_epoch(
    core: &Core,
    issuer: &TokenIssuer<'_>,
    record: &RegistrationRecord,
    client_address: Option<&str>,
) -> TokenError {
    if let Err(error) = core
        .end_registration(
            issuer,
            record,
            client_address,
            &[
                ("reason", "epoch_changed".to_owned()),
                ("actor", "system".to_owned()),
            ],
        )
        .await
    {
        tracing::error!(error = %error, registration_id = %record.id, "ending an agent registration after an epoch change failed");
    }
    TokenError::invalid_grant(
        "the user signed out everywhere or changed their password; claim the agent again",
    )
}

/// The user a bound registration acts for; `invalid_grant` when the user
/// no longer exists.
async fn bound_user(
    core: &Core,
    record: &RegistrationRecord,
) -> Result<Option<AgentUser>, TokenError> {
    let Some(user_id) = record.user_id.as_deref() else {
        return Ok(None);
    };
    match core.users.find_by_id(user_id).await {
        Ok(Some(user)) => Ok(Some(user)),
        Ok(None) => Err(TokenError::invalid_grant(
            "the user this registration is bound to no longer exists",
        )),
        Err(error) => Err(storage(&error)),
    }
}

/// The claim grant: an agent polls for the end of the claim ceremony.
pub(crate) struct ClaimGrant {
    core: Arc<Core>,
}

impl ClaimGrant {
    pub(crate) fn new(core: Arc<Core>) -> Self {
        Self { core }
    }
}

fn pending() -> TokenError {
    TokenError::new(
        400,
        "authorization_pending",
        Some("The user has not yet completed the ceremony.".to_owned()),
    )
}

fn expired_token(description: &str) -> TokenError {
    TokenError::new(400, "expired_token", Some(description.to_owned()))
}

/// When a claimed registration's claim token stops working: the end of the
/// registration window, or one user-code lifetime (ten minutes by default)
/// after the confirmation if that is later, so a claim made late in the
/// window can still be collected. A claimed token that is never polled
/// expires either way.
fn claimed_token_deadline(core: &Core, record: &RegistrationRecord) -> Option<DateTime<Utc>> {
    let after_claim = record
        .claimed_at
        .and_then(|claimed| super::later(claimed, core.config.user_code_lifetime).ok());
    match (record.claim_expires_at, after_claim) {
        (Some(window), Some(after)) => Some(window.max(after)),
        (window, after) => window.or(after),
    }
}

/// Take back an access token a poll minted but must not hand out.
async fn withdraw(issuer: &TokenIssuer<'_>, grant_id: &str, response: &TokenResponse) {
    use secrecy::ExposeSecret;
    if let Err(error) = issuer
        .revoke_token(grant_id, response.access_token.expose_secret())
        .await
    {
        tracing::error!(error = %error, "taking back an unclaimed access token failed");
    }
}

/// The whole second of `now`: every engine stores the poll time at that
/// resolution, so the interval is compared in whole seconds everywhere.
fn whole_second(now: DateTime<Utc>) -> DateTime<Utc> {
    DateTime::from_timestamp(now.timestamp(), 0).unwrap_or(now)
}

#[async_trait]
impl GrantHandler for ClaimGrant {
    fn grant_type(&self) -> &str {
        CLAIM_GRANT
    }

    async fn exchange(
        &self,
        request: &TokenRequest,
        issuer: &TokenIssuer<'_>,
    ) -> Result<TokenResponse, TokenError> {
        let core = &self.core;
        let claim_token = request
            .param("claim_token")
            .ok_or_else(|| TokenError::invalid_request("claim_token is required"))?;
        if claim_token.len() > MAX_PARAMETER_LENGTH {
            return Err(expired_token("The claim ceremony window has closed."));
        }
        let token_hash = digest(claim_token);
        let Some(record) = core
            .store
            .find_by_claim_token(&token_hash)
            .await
            .map_err(|error| storage(&error))?
        else {
            return Err(expired_token("The claim ceremony window has closed."));
        };
        let now = Utc::now();
        if record.status == STATUS_REVOKED {
            return Err(TokenError::invalid_grant("the registration was revoked"));
        }
        let Some(resource) = core.accepted_resource(
            request
                .param("resource")
                .unwrap_or(core.default_resource.as_str()),
        ) else {
            return Err(TokenError::invalid_target(
                "the resource is not served here",
            ));
        };
        if record.cleanup_pending {
            // A claim whose revocation of the pre-claim grants failed is
            // finished before anything is issued.
            core.finish_pending_cleanup(issuer, &record, request.client_address())
                .await;
        }
        let claimed = record.status == STATUS_CLAIMED;
        if record.status == STATUS_EXPIRED
            || (!claimed && record.claim_expires_at.is_none_or(|window| window <= now))
            || (claimed && claimed_token_deadline(core, &record).is_none_or(|end| end <= now))
        {
            return Err(expired_token("The claim ceremony window has closed."));
        }
        let slot = whole_second(now);
        let interval = chrono::Duration::from_std(core.config.poll_interval)
            .unwrap_or_else(|_| chrono::Duration::seconds(5));
        let earliest_previous = slot.checked_sub_signed(interval).unwrap_or(slot);
        if !core
            .store
            .record_poll(&record.id, slot, earliest_previous)
            .await
            .map_err(|error| storage(&error))?
        {
            return Err(TokenError::new(
                400,
                "slow_down",
                Some(format!(
                    "Polling too fast; wait at least {} seconds between polls.",
                    core.config.poll_interval.as_secs()
                )),
            ));
        }
        if !claimed {
            let Some(code_expiry) = record.user_code_expires_at else {
                // No attempt was started yet: an anonymous registration
                // waits for the agent to call the claim endpoint.
                return Err(pending());
            };
            if code_expiry <= now
                || record.user_code_failures >= core.config.max_user_code_failures
                || record.attempt_token_hash.is_none()
            {
                return Err(expired_token(
                    "The user code expired; call the claim endpoint again for a new one.",
                ));
            }
            return Err(pending());
        }
        let user = bound_user(core, &record).await?;
        if let Some(user) = &user
            && Core::epoch_moved(&record, user)
        {
            return Err(end_for_epoch(core, issuer, &record, request.client_address()).await);
        }
        // Issue first and spend the claim token last: a poll that issues
        // nothing (another resource, a storage failure) leaves the token
        // for the next poll.
        let (mut response, grant_id) = core
            .issue_for_registration(
                issuer,
                &record,
                user.as_ref(),
                core.bound_scopes(&record),
                core.bound_scopes(&record),
                resource,
            )
            .await?;
        let signed = core.assertion(&record, user.as_ref(), now);
        // The claim token is single use: of two polls that issued, the
        // database lets one spend it, and the other takes back the access
        // token it minted.
        let spent = match &signed {
            Ok(_) => core.store.spend_claim_token(&record.id, &token_hash).await,
            Err(_) => Ok(false),
        };
        let (assertion, expires) = match (signed, spent) {
            (Ok(signed), Ok(true)) => signed,
            (signed, spent) => {
                withdraw(issuer, &grant_id, &response).await;
                if let Err(error) = signed {
                    return Err(storage(&error));
                }
                if let Err(error) = spent {
                    return Err(storage(&error));
                }
                return Err(expired_token("The claim ceremony window has closed."));
            }
        };
        response
            .extra
            .insert("identity_assertion".to_owned(), Value::String(assertion));
        response.extra.insert(
            "assertion_expires".to_owned(),
            Value::String(timestamp(expires)),
        );
        let user_id = user.as_ref().map(|user| user.user_id.as_str());
        core.emit(
            LifecycleEventKind::AgentTokenIssued,
            user_id,
            &[
                ("registration_id", record.id.clone()),
                ("scope", response.scope.clone()),
                ("grant_type", CLAIM_GRANT.to_owned()),
            ],
            request.client_address(),
        )
        .await;
        core.emit(
            LifecycleEventKind::AgentAssertionIssued,
            user_id,
            &[("registration_id", record.id.clone())],
            request.client_address(),
        )
        .await;
        Ok(response)
    }
}

/// Records `token.revoked` for every agent token the revocation endpoint
/// ends.
pub(crate) struct RevocationRecorder {
    core: Arc<Core>,
}

impl RevocationRecorder {
    pub(crate) fn new(core: Arc<Core>) -> Self {
        Self { core }
    }
}

#[async_trait]
impl RevocationListener for RevocationRecorder {
    async fn token_revoked(&self, revoked: RevokedToken) {
        if revoked.grant_kind != AGENT_GRANT_KIND {
            return;
        }
        self.core
            .emit(
                LifecycleEventKind::AgentTokenRevoked,
                None,
                &[
                    ("registration_id", revoked.client_id),
                    ("token_kind", revoked.token_kind),
                ],
                revoked.client_address.as_deref(),
            )
            .await;
    }
}
