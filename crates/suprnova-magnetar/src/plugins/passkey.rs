//! The `WebAuthnAuthProvider` route plugin: registration and sign-in
//! ceremonies over the wire.
//!
//! Selector placement is the host adapter's choice: these routes carry the
//! opaque selector in the wire response and accept it back (the api-lane
//! capability); the Suprnova web adapter may instead keep it in its data
//! session and inject it into the forwarded request. The binding
//! invariants live in [`crate::passkey::PasskeyAuthService`] regardless of
//! placement.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::json;

use crate::abuse::AbusePolicy;
use crate::auth::SignInDecision;
use crate::passkey::{PasskeyAuthService, RegistrationIntent};
use crate::password::normalize_email;
use crate::plugin::{
    Effect, EffectResponse, Method, Plugin, PluginResult, RequestContext, RouteDescriptor,
    WireBody, WireResponse,
};
use crate::schema::AuthSchema;
use crate::sessions::VerifiedSession;
use crate::storage::CredentialActor;
use crate::{Error, Result};

use super::{Gate, acquire, bad_request, body_string, generic_ok, request_metadata};

/// Host boundary answering when a session's owner last confirmed their
/// password. The stamp feeds the three-hour reauth window on
/// existing-account enrollment; hosts read it from their data session.
#[async_trait]
pub trait ReauthSource: Send + Sync {
    /// The `password_confirmed_at` stamp for this session, when any.
    async fn password_confirmed_at(
        &self,
        session: &VerifiedSession,
    ) -> Result<Option<DateTime<Utc>>>;
}

/// Route-level configuration for the passkey plugin.
#[derive(Clone, Copy, Debug)]
pub struct PasskeyPluginConfig {
    /// Abuse budget for beginning registration ceremonies.
    pub register_policy: AbusePolicy,
    /// Abuse budget for beginning authentication ceremonies.
    pub login_policy: AbusePolicy,
}

impl Default for PasskeyPluginConfig {
    fn default() -> Self {
        Self {
            register_policy: AbusePolicy {
                max_requests: 10,
                window: std::time::Duration::from_secs(3600),
            },
            login_policy: AbusePolicy {
                max_requests: 10,
                window: std::time::Duration::from_secs(60),
            },
        }
    }
}

/// The passkey route plugin.
pub struct PasskeyPlugin {
    service: Arc<PasskeyAuthService>,
    reauth: Arc<dyn ReauthSource>,
    config: PasskeyPluginConfig,
}

impl PasskeyPlugin {
    /// Compose the plugin over the ceremony service and the host's reauth
    /// boundary.
    pub fn new(
        service: Arc<PasskeyAuthService>,
        reauth: Arc<dyn ReauthSource>,
        config: PasskeyPluginConfig,
    ) -> Self {
        Self {
            service,
            reauth,
            config,
        }
    }
}

fn body_value(request: &crate::plugin::WireRequest, field: &str) -> Option<serde_json::Value> {
    match &request.body {
        WireBody::Json(value) => value.get(field).cloned(),
        _ => None,
    }
}

fn map_error(error: Error) -> WireResponse {
    const AUTHENTICATION_FAILED: &str = "passkey authentication failed";
    let (status, message) = match &error {
        Error::InvalidInput { field, message } => match field.as_str() {
            "credentials" | "credential" | "actor" => (401, AUTHENTICATION_FAILED.to_owned()),
            "reauth" => (403, message.clone()),
            _ => (400, message.clone()),
        },
        Error::NotFound { resource, .. } if resource == "credential actor" => {
            (401, AUTHENTICATION_FAILED.to_owned())
        }
        Error::Conflict { message, .. } => (409, message.clone()),
        _ => (500, "internal error".to_owned()),
    };
    let mut response = EffectResponse::json(json!({"message": message}));
    response.status = status;
    WireResponse::from_effects(response)
}

#[async_trait]
impl<S: AuthSchema> Plugin<S> for PasskeyPlugin {
    fn name(&self) -> &str {
        "passkey"
    }

    fn routes(&self) -> Vec<RouteDescriptor> {
        vec![
            RouteDescriptor::new(
                Method::Post,
                "/passkeys/register/options",
                "passkey.register.options",
            )
            .with_feature("passkey"),
            RouteDescriptor::new(Method::Post, "/passkeys/register", "passkey.register")
                .with_feature("passkey"),
            RouteDescriptor::new(
                Method::Post,
                "/passkeys/login/options",
                "passkey.login.options",
            )
            .with_feature("passkey"),
            RouteDescriptor::new(Method::Post, "/passkeys/login", "passkey.login")
                .with_feature("passkey"),
        ]
    }

    async fn handle(&self, context: RequestContext<'_, S>) -> PluginResult<WireResponse> {
        match context.request.path.trim_matches('/') {
            "passkeys/register/options" => {
                let Some(email) = body_string(context.request, "email") else {
                    return Ok(bad_request("email is required"));
                };
                let identity = normalize_email(&email);
                match acquire(
                    &context,
                    "passkey.register",
                    &identity,
                    self.config.register_policy,
                )
                .await
                {
                    Gate::Proceed => {}
                    Gate::Respond(response) => return Ok(response),
                }
                let (actor, reauthenticated_at) = match context.session {
                    Some(session) => (
                        Some(CredentialActor::from_session(session)),
                        self.reauth.password_confirmed_at(session).await?,
                    ),
                    None => (None, None),
                };
                match self
                    .service
                    .begin_registration(RegistrationIntent {
                        email,
                        actor,
                        reauthenticated_at,
                    })
                    .await
                {
                    Ok(begun) => Ok(WireResponse::from_effects(EffectResponse::json(json!({
                        "selector": begun.selector,
                        "options": begun.options,
                    })))),
                    Err(error) => Ok(map_error(error)),
                }
            }
            "passkeys/register" => {
                let (Some(selector), Some(email), Some(credential)) = (
                    body_string(context.request, "selector"),
                    body_string(context.request, "email"),
                    body_value(context.request, "credential"),
                ) else {
                    return Ok(bad_request("selector, email, and credential are required"));
                };
                let credential = match serde_json::from_value(credential) {
                    Ok(credential) => credential,
                    Err(_) => return Ok(bad_request("credential is not a WebAuthn response")),
                };
                match self
                    .service
                    .finish_registration(&selector, &email, &credential)
                    .await
                {
                    Ok(_) => Ok(WireResponse::from_effects(EffectResponse::json(
                        generic_ok(),
                    ))),
                    Err(error) => Ok(map_error(error)),
                }
            }
            "passkeys/login/options" => {
                let Some(email) = body_string(context.request, "email") else {
                    return Ok(bad_request("email is required"));
                };
                let identity = normalize_email(&email);
                match acquire(
                    &context,
                    "passkey.login",
                    &identity,
                    self.config.login_policy,
                )
                .await
                {
                    Gate::Proceed => {}
                    Gate::Respond(response) => return Ok(response),
                }
                match self.service.begin_authentication(&email).await {
                    Ok(begun) => Ok(WireResponse::from_effects(EffectResponse::json(json!({
                        "selector": begun.selector,
                        "options": begun.options,
                    })))),
                    Err(error) => Ok(map_error(error)),
                }
            }
            "passkeys/login" => {
                let (Some(selector), Some(email), Some(credential)) = (
                    body_string(context.request, "selector"),
                    body_string(context.request, "email"),
                    body_value(context.request, "credential"),
                ) else {
                    return Ok(bad_request("selector, email, and credential are required"));
                };
                let credential = match serde_json::from_value(credential) {
                    Ok(credential) => credential,
                    Err(_) => return Ok(bad_request("credential is not a WebAuthn response")),
                };
                let metadata = request_metadata(context.request);
                match self
                    .service
                    .finish_authentication(&selector, &email, &credential, metadata)
                    .await
                {
                    Ok(SignInDecision::SessionAllowed(grant)) => Ok(WireResponse::from_effects(
                        EffectResponse::json(generic_ok())
                            .with_effect(Effect::EstablishSession(grant)),
                    )),
                    Ok(SignInDecision::FactorRequired { challenge_selector }) => {
                        Ok(WireResponse::from_effects(EffectResponse::json(json!({
                            "two_factor_required": true,
                            "challenge_selector": challenge_selector,
                        }))))
                    }
                    Err(error) => Ok(map_error(error)),
                }
            }
            other => Err(crate::plugin::PluginError::RouteNotFound {
                path: other.to_owned(),
            }),
        }
    }
}

/// A reauth source for hosts whose adapters have no password-confirmation
/// stamp (pure-API lanes): existing-account enrollment is always refused
/// until the host supplies a real boundary.
pub struct NoReauth;

#[async_trait]
impl ReauthSource for NoReauth {
    async fn password_confirmed_at(
        &self,
        _session: &VerifiedSession,
    ) -> Result<Option<DateTime<Utc>>> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_actor_is_indistinguishable_from_other_credential_failures() {
        let stale = map_error(Error::NotFound {
            resource: "credential actor".to_owned(),
            identifier: "expired or revoked".to_owned(),
        });
        let invalid = map_error(Error::InvalidInput {
            field: "credential".to_owned(),
            message: "sensitive verification detail".to_owned(),
        });

        assert_eq!(stale.0.status, 401);
        assert_eq!(stale.0.body, invalid.0.body);
        assert_eq!(
            stale.0.body,
            Some(json!({"message": "passkey authentication failed"}))
        );
    }
}
