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
        let replay_until = DateTime::from_timestamp(iat, 0)
            .and_then(|issued| later(issued, self.config.event_max_age).ok())
            .and_then(|until| later(until, self.config.clock_skew).ok())
            .unwrap_or(now);
        if !self
            .store
            .record_replay(&digest(&format!("set\n{issuer}\n{jti}")), replay_until)
            .await?
        {
            return Ok(set_error(
                "invalid_request",
                "this event was already received",
            ));
        }
        // Unknown event schemas are ignored (RFC 8417 section 2.2).
        if revokes && let Some(subject) = subject {
            let delegation = digest(&format!("{issuer}\n{subject}"));
            if let Some(record) = self.store.find_by_delegation(&delegation).await? {
                self.end_registration(
                    server,
                    &record,
                    client_address,
                    &[("reason", "security_event".to_owned())],
                )
                .await?;
            }
        }
        Ok(respond(202, None, &NO_STORE))
    }
}
