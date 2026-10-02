//! First-party plugins built on the public SDK.
//!
//! Every plugin here is mirrored one-to-one by a Cargo feature, so an unused
//! plugin costs nothing in the binary. Plugins consume only the same public
//! surface offered to third-party authors, plus the crate-internal privilege
//! of minting [`crate::auth::VerifiedPrincipal`] at their primary-auth
//! boundaries.

#[cfg(feature = "device-authorization")]
pub mod device_authorization;
#[cfg(feature = "email-verification")]
pub mod email_verification;
#[cfg(feature = "magic-link")]
pub mod magic_link;
#[cfg(feature = "oauth-apple")]
pub mod oauth_apple;
#[cfg(feature = "oauth-facebook")]
pub mod oauth_facebook;
#[cfg(feature = "oauth-google")]
pub mod oauth_google;
#[cfg(feature = "oauth-tiktok")]
pub mod oauth_tiktok;
#[cfg(feature = "oauth-x")]
pub mod oauth_x;
#[cfg(feature = "passkey")]
pub mod passkey;
#[cfg(feature = "password")]
pub mod password;
#[cfg(feature = "password-management")]
pub mod password_management;
#[cfg(feature = "two-factor")]
pub mod two_factor;

#[cfg(any(
    feature = "password",
    feature = "email-verification",
    feature = "password-management",
    feature = "magic-link",
    feature = "passkey",
    feature = "two-factor",
    feature = "device-authorization"
))]
pub use shared::abuse_key;
#[cfg(any(
    feature = "password",
    feature = "magic-link",
    feature = "passkey",
    feature = "device-authorization"
))]
pub(crate) use shared::request_metadata;
#[cfg(feature = "password")]
pub(crate) use shared::unavailable;
#[cfg(any(
    feature = "password",
    feature = "email-verification",
    feature = "password-management",
    feature = "magic-link",
    feature = "passkey",
    feature = "device-authorization"
))]
pub(crate) use shared::{Gate, acquire};
#[cfg(any(
    feature = "password",
    feature = "email-verification",
    feature = "password-management",
    feature = "magic-link",
    feature = "passkey",
    feature = "two-factor",
    feature = "device-authorization"
))]
pub(crate) use shared::{bad_request, body_string, generic_ok};

/// Route helpers shared by every first-party plugin.
#[cfg(any(
    feature = "password",
    feature = "email-verification",
    feature = "password-management",
    feature = "magic-link",
    feature = "passkey",
    feature = "two-factor",
    feature = "device-authorization"
))]
mod shared {
    use serde_json::json;

    #[cfg(any(
        feature = "password",
        feature = "email-verification",
        feature = "password-management",
        feature = "magic-link",
        feature = "passkey",
        feature = "device-authorization"
    ))]
    use crate::abuse::{AbusePolicy, Permit};
    #[cfg(any(
        feature = "password",
        feature = "email-verification",
        feature = "password-management",
        feature = "magic-link",
        feature = "passkey",
        feature = "device-authorization"
    ))]
    use crate::plugin::{Effect, RequestContext};
    use crate::plugin::{EffectResponse, WireBody, WireRequest, WireResponse};
    #[cfg(any(
        feature = "password",
        feature = "email-verification",
        feature = "password-management",
        feature = "magic-link",
        feature = "passkey",
        feature = "device-authorization"
    ))]
    use crate::schema::AuthSchema;

    /// Session metadata drawn from carrier-neutral request headers.
    ///
    /// Host adapters populate `user-agent` and `x-client-ip` on the
    /// [`WireRequest`] they forward; both stay optional.
    #[cfg(any(
        feature = "password",
        feature = "magic-link",
        feature = "passkey",
        feature = "device-authorization"
    ))]
    pub(crate) fn request_metadata(request: &WireRequest) -> crate::sessions::SessionMetadata {
        crate::sessions::SessionMetadata {
            user_agent: request.headers.get("user-agent").cloned(),
            ip_address: request.headers.get("x-client-ip").cloned(),
        }
    }

    /// Read one required string field from a JSON or form request body.
    pub(crate) fn body_string(request: &WireRequest, field: &str) -> Option<String> {
        match &request.body {
            WireBody::Json(value) => value
                .get(field)
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned),
            WireBody::Form(fields) => fields.get(field).cloned(),
            _ => None,
        }
    }

    /// Build a purpose-scoped abuse-limiter key from a normalized identity.
    ///
    /// The identity is digested so raw addresses never reach the limiter
    /// backend; the purpose prefix keeps every route on its own budget.
    pub fn abuse_key(purpose: &str, identity: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(identity.as_bytes());
        let digest = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        format!("{purpose}:{digest}")
    }

    #[cfg(any(
        feature = "password",
        feature = "email-verification",
        feature = "password-management",
        feature = "magic-link",
        feature = "passkey",
        feature = "device-authorization"
    ))]
    /// Outcome of the shared abuse-limiter gate.
    pub(crate) enum Gate {
        /// Budget consumed; continue.
        Proceed,
        /// Terminal response (over budget or backend failure, failing closed).
        Respond(WireResponse),
    }

    #[cfg(any(
        feature = "password",
        feature = "email-verification",
        feature = "password-management",
        feature = "magic-link",
        feature = "passkey",
        feature = "device-authorization"
    ))]
    /// Acquire one purpose-scoped abuse permit. The exact same acquisition
    /// runs for present and absent identities, and a limiter backend failure
    /// fails closed without revealing account existence.
    pub(crate) async fn acquire<S: AuthSchema>(
        context: &RequestContext<'_, S>,
        purpose: &str,
        identity: &str,
        policy: AbusePolicy,
    ) -> Gate {
        let key = abuse_key(purpose, identity);
        match context.plugin.abuse_limiter().acquire(&key, policy).await {
            Ok(Permit::Allowed { .. }) => Gate::Proceed,
            Ok(Permit::Rejected { retry_after }) => {
                let mut response = EffectResponse::json(json!({
                    "message": "too many requests",
                }))
                .with_effect(Effect::SetHeader {
                    name: "retry-after".to_owned(),
                    value: retry_after.as_secs().max(1).to_string(),
                });
                response.status = 429;
                Gate::Respond(WireResponse::from_effects(response))
            }
            Err(error) => {
                tracing::error!(
                    error = %error,
                    purpose,
                    "abuse limiter unavailable; failing closed"
                );
                Gate::Respond(unavailable())
            }
        }
    }

    /// The generic success body shared by anti-enumeration routes.
    pub(crate) fn generic_ok() -> serde_json::Value {
        json!({"status": "ok"})
    }

    /// A 400 response with a caller-safe message.
    pub(crate) fn bad_request(message: &str) -> WireResponse {
        let mut response = EffectResponse::json(json!({"message": message}));
        response.status = 400;
        WireResponse::from_effects(response)
    }

    #[cfg(any(
        feature = "password",
        feature = "email-verification",
        feature = "password-management",
        feature = "magic-link",
        feature = "passkey",
        feature = "device-authorization"
    ))]
    /// The fail-closed 503 response for unavailable dependencies.
    pub(crate) fn unavailable() -> WireResponse {
        let mut response = EffectResponse::json(json!({"message": "service unavailable"}));
        response.status = 503;
        WireResponse::from_effects(response)
    }
}
