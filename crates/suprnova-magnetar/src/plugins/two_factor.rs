//! The two-factor route plugin: enrollment lifecycle and the challenge.
//!
//! Enrollment routes require the authenticated owner; the challenge route
//! is deliberately unauthenticated — it completes the gate ceremony a
//! primary sign-in created, and only the gate can turn that proof into a
//! session. The remember-me preference stated at login rides the challenge
//! request (the host adapter injects the stashed value on the web lane),
//! so remember-me-with-2FA users get the same issuance the deployed
//! `complete_challenge` performs.

use std::sync::Arc;

use async_trait::async_trait;
use secrecy::ExposeSecret;
use serde_json::json;

use crate::Error;
use crate::plugin::{
    Effect, EffectResponse, Method, Plugin, PluginResult, RequestContext, RouteDescriptor,
    WireBody, WireResponse,
};
use crate::schema::AuthSchema;
use crate::sessions::RememberFacade;
use crate::storage::CredentialActor;
use crate::two_factor::TwoFactorService;

use super::{bad_request, body_string, generic_ok};

/// The two-factor route plugin.
pub struct TwoFactorPlugin {
    service: Arc<TwoFactorService>,
    remember: Option<Arc<dyn RememberFacade>>,
}

impl TwoFactorPlugin {
    /// Compose the plugin over the lifecycle service; the remember boundary
    /// honors login-time remember-me preferences on challenge completion.
    pub fn new(service: Arc<TwoFactorService>, remember: Option<Arc<dyn RememberFacade>>) -> Self {
        Self { service, remember }
    }
}

fn unauthenticated() -> WireResponse {
    let mut response = EffectResponse::json(json!({"message": "unauthenticated"}));
    response.status = 401;
    WireResponse::from_effects(response)
}

fn map_error(error: Error) -> WireResponse {
    let (status, message) = match &error {
        Error::InvalidInput { field, message } => match field.as_str() {
            "code" | "proof" => (401, message.clone()),
            _ => (400, message.clone()),
        },
        Error::Conflict { resource, message } if resource == "account lockout" => {
            (429, message.clone())
        }
        Error::Conflict { message, .. } => (409, message.clone()),
        Error::NotFound { resource, .. } if resource == "credential actor" => {
            (401, "two-factor authentication failed".to_owned())
        }
        Error::NotFound { .. } => (400, "invalid or expired challenge".to_owned()),
        _ => (500, "internal error".to_owned()),
    };
    let mut response = EffectResponse::json(json!({"message": message}));
    response.status = status;
    WireResponse::from_effects(response)
}

fn wants_remember(request: &crate::plugin::WireRequest) -> bool {
    match &request.body {
        WireBody::Json(value) => value
            .get("remember")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        WireBody::Form(fields) => fields
            .get("remember")
            .is_some_and(|value| value == "true" || value == "1" || value == "on"),
        _ => false,
    }
}

#[async_trait]
impl<S: AuthSchema> Plugin<S> for TwoFactorPlugin {
    fn name(&self) -> &str {
        "two-factor"
    }

    fn routes(&self) -> Vec<RouteDescriptor> {
        vec![
            RouteDescriptor::new(Method::Post, "/user/two-factor", "two-factor.enroll")
                .with_feature("two-factor"),
            RouteDescriptor::new(
                Method::Post,
                "/user/two-factor/confirm",
                "two-factor.confirm",
            )
            .with_feature("two-factor"),
            RouteDescriptor::new(Method::Delete, "/user/two-factor", "two-factor.disable")
                .with_feature("two-factor"),
            RouteDescriptor::new(
                Method::Post,
                "/user/two-factor/recovery-codes",
                "two-factor.recovery-codes",
            )
            .with_feature("two-factor"),
            RouteDescriptor::new(
                Method::Post,
                "/two-factor-challenge",
                "two-factor.challenge",
            )
            .with_feature("two-factor"),
        ]
    }

    async fn handle(&self, context: RequestContext<'_, S>) -> PluginResult<WireResponse> {
        let path = context.request.path.trim_matches('/');

        if path == "two-factor-challenge" {
            let (Some(selector), Some(code)) = (
                body_string(context.request, "challenge_selector"),
                body_string(context.request, "code"),
            ) else {
                return Ok(bad_request("challenge_selector and code are required"));
            };
            return match context
                .plugin
                .factor_gate()
                .complete_challenge(&selector, &code)
                .await
            {
                Ok(grant) => {
                    let user_id = grant.user_id().to_owned();
                    let mut response = EffectResponse::json(generic_ok())
                        .with_effect(Effect::EstablishSession(grant));
                    if wants_remember(context.request)
                        && let Some(remember) = &self.remember
                    {
                        response = response.with_effect(Effect::IssueRemember(
                            remember.issue_now(&user_id).await?,
                        ));
                    }
                    Ok(WireResponse::from_effects(response))
                }
                Err(error) => Ok(map_error(error)),
            };
        }

        // Every lifecycle route acts on the authenticated owner only.
        let Some(session) = context.session else {
            return Ok(unauthenticated());
        };
        let actor = CredentialActor::from_session(session);

        match (path, &context.request.method) {
            ("user/two-factor", Method::Post) => match self.service.enroll(&actor).await {
                Ok(enrollment) => Ok(WireResponse::from_effects(EffectResponse::json(json!({
                    // Shown exactly once; there is no retrieval API.
                    "otpauth_url": enrollment.otpauth_url.expose_secret(),
                    "qr_code_svg": enrollment.qr_code_svg,
                    "recovery_codes": enrollment.recovery_codes,
                })))),
                Err(error) => Ok(map_error(error)),
            },
            ("user/two-factor", Method::Delete) => match self.service.disable(&actor).await {
                Ok(_) => Ok(WireResponse::from_effects(EffectResponse::json(
                    generic_ok(),
                ))),
                Err(error) => Ok(map_error(error)),
            },
            ("user/two-factor/confirm", Method::Post) => {
                let Some(code) = body_string(context.request, "code") else {
                    return Ok(bad_request("code is required"));
                };
                match self.service.confirm(&actor, &code).await {
                    Ok(()) => Ok(WireResponse::from_effects(EffectResponse::json(
                        generic_ok(),
                    ))),
                    Err(error) => Ok(map_error(error)),
                }
            }
            ("user/two-factor/recovery-codes", Method::Post) => {
                let Some(proof) = body_string(context.request, "proof") else {
                    return Ok(bad_request("proof is required"));
                };
                match self.service.regenerate_recovery_codes(&actor, &proof).await {
                    Ok(codes) => Ok(WireResponse::from_effects(EffectResponse::json(json!({
                        "recovery_codes": codes,
                    })))),
                    Err(error) => Ok(map_error(error)),
                }
            }
            (other, _) => Err(crate::plugin::PluginError::RouteNotFound {
                path: other.to_owned(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_actor_is_mapped_to_a_generic_authentication_failure() {
        let response = map_error(Error::NotFound {
            resource: "credential actor".to_owned(),
            identifier: "expired or revoked".to_owned(),
        });

        assert_eq!(response.0.status, 401);
        assert_eq!(
            response.0.body,
            Some(json!({"message": "two-factor authentication failed"}))
        );
    }
}
