//! Mounted first-party device sign-in routes.

use std::sync::Arc;

use async_trait::async_trait;
use secrecy::ExposeSecret;
use serde_json::json;

use crate::Error;
use crate::abuse::AbusePolicy;
use crate::oauth::device::{
    DeviceApprovalOutcome, DeviceAuthorizationService, DeviceCeremonyStatus, DevicePollOutcome,
};
use crate::plugin::{
    Effect, EffectResponse, Method, Plugin, PluginResult, RequestContext, RouteDescriptor,
    WireResponse,
};
use crate::schema::AuthSchema;
use crate::storage::CredentialActor;

use super::{Gate, acquire, bad_request, body_string, generic_ok, request_metadata};

/// Route configuration for [`DeviceAuthorizationPlugin`].
#[derive(Clone, Debug)]
pub struct DeviceAuthorizationPluginConfig {
    /// Prefix under which the six device sign-in routes are mounted.
    pub route_prefix: String,
    /// Abuse budget for creating canonical and poll ceremony rows.
    pub issue_policy: AbusePolicy,
}

impl Default for DeviceAuthorizationPluginConfig {
    fn default() -> Self {
        Self {
            route_prefix: "/oauth/device".to_owned(),
            issue_policy: AbusePolicy {
                max_requests: 10,
                window: std::time::Duration::from_secs(60),
            },
        }
    }
}

/// First-party device sign-in route group.
pub struct DeviceAuthorizationPlugin {
    service: Arc<DeviceAuthorizationService>,
    config: DeviceAuthorizationPluginConfig,
}

impl DeviceAuthorizationPlugin {
    /// Compose the route group over a configured device service.
    #[must_use]
    pub fn new(
        service: Arc<DeviceAuthorizationService>,
        config: DeviceAuthorizationPluginConfig,
    ) -> Self {
        Self { service, config }
    }

    fn route(&self, suffix: &str) -> String {
        format!(
            "{}/{}",
            self.config.route_prefix.trim_end_matches('/'),
            suffix.trim_start_matches('/')
        )
    }
}

fn unauthenticated() -> WireResponse {
    let mut response = EffectResponse::json(json!({"message": "unauthenticated"}));
    response.status = 401;
    WireResponse::from_effects(response)
}

fn map_error(error: Error) -> WireResponse {
    let (status, message) = match error {
        Error::InvalidInput { message, .. } => (400, message),
        Error::NotFound { resource, .. } if resource == "credential actor" => {
            (401, "authentication failed".to_owned())
        }
        Error::NotFound { .. } => (400, "invalid or expired device code".to_owned()),
        Error::Conflict { message, .. } => (409, message),
        _ => (500, "internal error".to_owned()),
    };
    let mut response = EffectResponse::json(json!({"message": message}));
    response.status = status;
    WireResponse::from_effects(response)
}

fn actor<S: AuthSchema>(context: &RequestContext<'_, S>) -> Option<CredentialActor> {
    context.session.map(CredentialActor::from_session)
}

#[async_trait]
impl<S: AuthSchema> Plugin<S> for DeviceAuthorizationPlugin {
    fn name(&self) -> &str {
        "device-authorization"
    }

    fn routes(&self) -> Vec<RouteDescriptor> {
        vec![
            RouteDescriptor::new(
                Method::Post,
                self.route("code"),
                "device-authorization.code",
            )
            .with_feature("device-authorization"),
            RouteDescriptor::new(
                Method::Post,
                self.route("verify"),
                "device-authorization.verify",
            )
            .with_feature("device-authorization"),
            RouteDescriptor::new(
                Method::Post,
                self.route("approve"),
                "device-authorization.approve",
            )
            .with_feature("device-authorization"),
            RouteDescriptor::new(
                Method::Post,
                self.route("approve/challenge"),
                "device-authorization.approve-challenge",
            )
            .with_feature("device-authorization"),
            RouteDescriptor::new(
                Method::Post,
                self.route("deny"),
                "device-authorization.deny",
            )
            .with_feature("device-authorization"),
            RouteDescriptor::new(
                Method::Post,
                self.route("poll"),
                "device-authorization.poll",
            )
            .with_feature("device-authorization"),
        ]
    }

    async fn handle(&self, context: RequestContext<'_, S>) -> PluginResult<WireResponse> {
        let path = context.request.path.trim_end_matches('/');

        if path == self.route("code") {
            let metadata = request_metadata(context.request);
            let identity = metadata
                .ip_address
                .as_deref()
                .or(metadata.user_agent.as_deref())
                .unwrap_or("anonymous");
            match acquire(
                &context,
                "device-code.issue",
                identity,
                self.config.issue_policy,
            )
            .await
            {
                Gate::Proceed => {}
                Gate::Respond(response) => return Ok(response),
            }
            return match self.service.issue_code().await {
                Ok(issued) => Ok(WireResponse::from_effects(EffectResponse::json(json!({
                    "device_code": issued.device_code.expose_secret(),
                    "user_code": issued.user_code,
                    "verification_uri": issued.verification_uri,
                    "verification_uri_complete": issued.verification_uri_complete,
                    "expires_in": issued.expires_in,
                    "interval": issued.interval,
                })))),
                Err(error) => Ok(map_error(error)),
            };
        }

        if path == self.route("verify") {
            let Some(user_code) = body_string(context.request, "user_code") else {
                return Ok(bad_request("user_code is required"));
            };
            return match self.service.verify(&user_code).await {
                Ok(display) => {
                    let status = match display.status {
                        DeviceCeremonyStatus::Pending => "pending",
                        DeviceCeremonyStatus::Approved => "approved",
                        DeviceCeremonyStatus::Denied => "denied",
                        DeviceCeremonyStatus::Issued => "issued",
                    };
                    Ok(WireResponse::from_effects(EffectResponse::json(json!({
                        "status": status,
                        "expires_at": display.expires_at,
                    }))))
                }
                Err(error) => Ok(map_error(error)),
            };
        }

        if path == self.route("approve") {
            let Some(credential_actor) = actor(&context) else {
                return Ok(unauthenticated());
            };
            let Some(user_code) = body_string(context.request, "user_code") else {
                return Ok(bad_request("user_code is required"));
            };
            return match self
                .service
                .approve(
                    &user_code,
                    &credential_actor,
                    request_metadata(context.request),
                )
                .await
            {
                Ok(DeviceApprovalOutcome::Approved) => Ok(WireResponse::from_effects(
                    EffectResponse::json(json!({"status": "approved"})),
                )),
                Ok(DeviceApprovalOutcome::FactorRequired { challenge_selector }) => {
                    Ok(WireResponse::from_effects(EffectResponse::json(json!({
                        "status": "factor_required",
                        "challenge_selector": challenge_selector,
                    }))))
                }
                Err(error) => Ok(map_error(error)),
            };
        }

        if path == self.route("approve/challenge") {
            let Some(credential_actor) = actor(&context) else {
                return Ok(unauthenticated());
            };
            let (Some(selector), Some(code)) = (
                body_string(context.request, "challenge_selector"),
                body_string(context.request, "code"),
            ) else {
                return Ok(bad_request("challenge_selector and code are required"));
            };
            return match self
                .service
                .complete_approval(&selector, &code, &credential_actor)
                .await
            {
                Ok(()) => Ok(WireResponse::from_effects(EffectResponse::json(
                    json!({"status": "approved"}),
                ))),
                Err(error) => Ok(map_error(error)),
            };
        }

        if path == self.route("deny") {
            let Some(credential_actor) = actor(&context) else {
                return Ok(unauthenticated());
            };
            let Some(user_code) = body_string(context.request, "user_code") else {
                return Ok(bad_request("user_code is required"));
            };
            return match self.service.deny(&user_code, &credential_actor).await {
                Ok(()) => Ok(WireResponse::from_effects(EffectResponse::json(
                    generic_ok(),
                ))),
                Err(error) => Ok(map_error(error)),
            };
        }

        if path == self.route("poll") {
            let Some(device_code) = body_string(context.request, "device_code") else {
                return Ok(bad_request("device_code is required"));
            };
            return match self.service.poll(&device_code).await {
                Ok(DevicePollOutcome::AuthorizationPending) => {
                    let mut response = EffectResponse::json(json!({
                        "status": "authorization_pending",
                    }));
                    response.status = 202;
                    Ok(WireResponse::from_effects(response))
                }
                Ok(DevicePollOutcome::SlowDown { interval }) => {
                    let mut response = EffectResponse::json(json!({
                        "status": "slow_down",
                        "interval": interval,
                    }));
                    response.status = 429;
                    Ok(WireResponse::from_effects(response))
                }
                Ok(DevicePollOutcome::AccessDenied) => {
                    let mut response = EffectResponse::json(json!({"status": "access_denied"}));
                    response.status = 403;
                    Ok(WireResponse::from_effects(response))
                }
                Ok(DevicePollOutcome::ExpiredToken) => {
                    let mut response = EffectResponse::json(json!({"status": "expired_token"}));
                    response.status = 400;
                    Ok(WireResponse::from_effects(response))
                }
                Ok(DevicePollOutcome::Success(grant)) => Ok(WireResponse::from_effects(
                    EffectResponse::json(generic_ok())
                        .with_effect(Effect::EstablishSession(*grant)),
                )),
                Err(error) => Ok(map_error(error)),
            };
        }

        Err(crate::plugin::PluginError::RouteNotFound {
            path: context.request.path.clone(),
        })
    }
}
