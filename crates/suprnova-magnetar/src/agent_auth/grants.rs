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
        let keys =
            std::iter::once(&core.config.signing_key).chain(&core.config.previous_signing_keys);
        let claims = verify_assertion(assertion, keys, &core.issuer, now)
            .map_err(TokenError::invalid_grant)?;
        let record = core
            .store
            .find_registration(&claims.sub)
            .await
            .map_err(|error| storage(&error))?
            .ok_or_else(|| TokenError::invalid_grant("the registration does not exist"))?;
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
        let allowed = if unclaimed {
            core.config.pre_claim_scopes.clone()
        } else {
            record.scopes.clone()
        };
        let scopes = match request.param("scope") {
            None => allowed,
            Some(scope) => {
                let requested: Vec<&str> = scope.split_whitespace().collect();
                if let Some(outside) = requested
                    .iter()
                    .find(|name| !allowed.iter().any(|allowed| allowed == *name))
                {
                    return Err(TokenError::invalid_scope(format!(
                        "scope {outside:?} is not held by this registration"
                    )));
                }
                allowed
                    .into_iter()
                    .filter(|name| requested.contains(&name.as_str()))
                    .collect()
            }
        };
        let user = bound_user(core, &record).await?;
        if let Some(user) = &user
            && (Core::epoch_moved(&record, user)
                || (user.auth_epoch.is_some() && claims.auth_epoch != user.auth_epoch))
        {
            return Err(end_for_epoch(core, issuer, &record, request.client_address()).await);
        }
        let resource = request
            .param("resource")
            .map_or_else(|| core.default_resource.clone(), str::to_owned);
        let response = core
            .issue_agent_grant(issuer, &record, user.as_ref(), scopes, resource)
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
/// stored: the assertion must not work, and a failed revocation is retried
/// by the next presentation, which finds the epoch still moved.
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
            &[("reason", "epoch_changed".to_owned())],
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
        let claimed = record.status == STATUS_CLAIMED;
        if record.status == STATUS_EXPIRED
            || (!claimed && record.claim_expires_at.is_none_or(|window| window <= now))
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
        // The claim token is single use: of two polls that see the claim,
        // the database lets one spend the token and the other answers as
        // for a token it does not know.
        if !core
            .store
            .spend_claim_token(&record.id, &token_hash)
            .await
            .map_err(|error| storage(&error))?
        {
            return Err(expired_token("The claim ceremony window has closed."));
        }
        let user = bound_user(core, &record).await?;
        if let Some(user) = &user
            && Core::epoch_moved(&record, user)
        {
            return Err(end_for_epoch(core, issuer, &record, request.client_address()).await);
        }
        let mut response = core
            .issue_agent_grant(
                issuer,
                &record,
                user.as_ref(),
                record.scopes.clone(),
                request
                    .param("resource")
                    .map_or_else(|| core.default_resource.clone(), str::to_owned),
            )
            .await?;
        let (assertion, expires) = core
            .assertion(&record, user.as_ref(), now)
            .map_err(|error| storage(&error))?;
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
