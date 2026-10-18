//! The events endpoint: Security Event Tokens (RFC 8417) a trusted
//! provider pushes (RFC 8935), verified like an ID-JAG, that revoke a
//! delegation and every token issued from it.

use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use super::jwt::{
    ProviderTokenError, SET_TYPE, audience_matches, numeric_date, verify_provider_token,
};
use super::{ASSERTION_REVOKED_EVENT, Core, NO_STORE, digest, later, respond};
use crate::Result;
use crate::authorization_server::AuthorizationServer;
use crate::plugin::WireResponse;

/// An RFC 8935 section 2.4 error answer.
fn set_error(error: &str, description: &str) -> WireResponse {
    respond(
        400,
        Some(json!({"err": error, "description": description})),
        &NO_STORE,
    )
}

impl Core {
    pub(crate) async fn receive_event(
        &self,
        server: &AuthorizationServer,
        content_type: Option<&str>,
        body: &[u8],
        client_address: Option<&str>,
    ) -> WireResponse {
        match self
            .receive_event_checked(server, content_type, body, client_address)
            .await
        {
            Ok(response) => response,
            Err(error) => {
                tracing::error!(error = %error, "receiving a security event failed");
                // RFC 8935 has no server error code; a 5xx tells the
                // transmitter to deliver again later.
                respond(500, None, &NO_STORE)
            }
        }
    }

    async fn receive_event_checked(
        &self,
        server: &AuthorizationServer,
        content_type: Option<&str>,
        body: &[u8],
        client_address: Option<&str>,
    ) -> Result<WireResponse> {
        // Unauthenticated callers reach this endpoint, and a token naming an
        // unseen key can cost a key set fetch: each address has a budget.
        if let Some(address) = client_address {
            match self
                .admit_one(
                    "agent-auth.events.address",
                    address,
                    self.config.limits.events_per_address,
                )
                .await
            {
                Ok(None) => {}
                Ok(Some(retry_after)) => {
                    let seconds = retry_after.as_secs().max(1).to_string();
                    return Ok(respond(
                        429,
                        Some(json!({
                            "err": "rate_limited",
                            "description": "too many events from this address; retry later",
                        })),
                        &[NO_STORE[0], NO_STORE[1], ("retry-after", seconds.as_str())],
                    ));
                }
                Err(error) => {
                    tracing::error!(error = %error, "abuse limiter unavailable; failing closed");
                    return Ok(respond(
                        503,
                        Some(json!({"err": "temporarily_unavailable"})),
                        &NO_STORE,
                    ));
                }
            }
        }
        let media = content_type
            .and_then(|value| value.split(';').next())
            .map(str::trim);
        if !media.is_some_and(|media| media.eq_ignore_ascii_case("application/secevent+jwt")) {
            return Ok(set_error(
                "invalid_request",
                "the body must be application/secevent+jwt",
            ));
        }
        let Ok(token) = std::str::from_utf8(body) else {
            return Ok(set_error("invalid_request", "the body is not a JWT"));
        };
        let verified = match verify_provider_token(
            token.trim(),
            SET_TYPE,
            &self.config.trusted_issuers,
            &self.key_sets,
        )
        .await
        {
            Ok(verified) => verified,
            Err(ProviderTokenError::Malformed(reason)) => {
                return Ok(set_error("invalid_request", reason));
            }
            Err(ProviderTokenError::UntrustedIssuer) => {
                return Ok(set_error(
                    "invalid_issuer",
                    "the issuer is not on this service's trust list",
                ));
            }
            Err(ProviderTokenError::KeysUnavailable(reason)) => {
                tracing::warn!(reason, "a trusted provider's key set could not be fetched");
                return Ok(set_error(
                    "invalid_key",
                    "the provider's key set could not be fetched",
                ));
            }
            Err(ProviderTokenError::UnknownKey) => {
                return Ok(set_error(
                    "invalid_key",
                    "the provider's key set has no key with this key id",
                ));
            }
            Err(ProviderTokenError::BadSignature) => {
                return Ok(set_error(
                    "authentication_failed",
                    "the signature does not verify against the provider's key",
                ));
            }
        };
        let claims = &verified.claims;
        if !audience_matches(claims, &self.audiences) {
            return Ok(set_error(
                "invalid_audience",
                "aud does not name this service",
            ));
        }
        let now = Utc::now();
        let skew = i64::try_from(self.config.clock_skew.as_secs()).unwrap_or(i64::MAX);
        let max_age = i64::try_from(self.config.event_max_age.as_secs()).unwrap_or(i64::MAX);
        let Some(iat) = numeric_date(claims, "iat") else {
            return Ok(set_error("invalid_request", "iat is required"));
        };
        if iat > now.timestamp().saturating_add(skew)
            || iat < now.timestamp().saturating_sub(max_age)
        {
            return Ok(set_error(
                "invalid_request",
                "iat is outside the window this service accepts",
            ));
        }
        if let Some(exp) = numeric_date(claims, "exp") {
            let max_ahead = i64::try_from(self.config.max_provider_token_lifetime.as_secs())
                .unwrap_or(i64::MAX);
            if exp <= now.timestamp() || exp > now.timestamp().saturating_add(max_ahead) {
                return Ok(set_error(
                    "invalid_request",
                    "exp is outside the window this service accepts",
                ));
            }
        }
        let jti = match claims.get("jti").and_then(Value::as_str) {
            Some(jti) if !jti.is_empty() && jti.len() <= 512 => jti,
            _ => return Ok(set_error("invalid_request", "jti is required")),
        };
        let Some(events) = claims.get("events").and_then(Value::as_object) else {
            return Ok(set_error("invalid_request", "events is required"));
        };
        let revokes = events.contains_key(ASSERTION_REVOKED_EVENT);
        let subject = claims.get("sub").and_then(Value::as_str);
        if revokes && subject.is_none_or(str::is_empty) {
            return Ok(set_error(
                "invalid_request",
                "a revocation event needs the subject in sub",
            ));
        }
        let issuer = verified.trusted.issuer.clone();
        let replay_key = digest(&format!("set\n{issuer}\n{jti}"));
        let now = Utc::now();
        if self.store.replay_recorded(&replay_key, now).await? {
            return Ok(set_error(
                "invalid_request",
                "this event was already received",
            ));
        }
        // The revocation runs before the identifier is recorded: it is
        // idempotent, and a failure here answers 500 with nothing recorded,
        // so the provider's redelivery revokes instead of being refused as
        // a replay. Unknown event schemas are ignored (RFC 8417 section
        // 2.2).
        if revokes && let Some(subject) = subject {
            let delegation = digest(&format!("{issuer}\n{subject}"));
            if let Some(record) = self.store.find_by_delegation(&delegation).await? {
                self.end_registration(
                    &server.token_issuer(),
                    &record,
                    client_address,
                    &[
                        ("reason", "security_event".to_owned()),
                        ("actor", "provider".to_owned()),
                    ],
                )
                .await?;
            }
        }
        let replay_until = DateTime::from_timestamp(iat, 0)
            .and_then(|issued| later(issued, self.config.event_max_age).ok())
            .and_then(|until| later(until, self.config.clock_skew).ok())
            .unwrap_or(now);
        // A concurrent delivery of the same event may record it first; both
        // revoked the same registration, so both are accepted.
        self.store.record_replay(&replay_key, replay_until).await?;
        Ok(respond(202, None, &NO_STORE))
    }
}
