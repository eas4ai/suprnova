//! Passkey registration and authentication through Magnetar ceremonies.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use super::{Session, User};
use crate::error::FrameworkError;
use crate::session::{session, session_mut};

pub use webauthn_rs::prelude::{
    AuthenticationResult as PasskeyAuthenticationResult, CreationChallengeResponse,
    PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential, RegisterPublicKeyCredential,
    RequestChallengeResponse,
};

const SESSION_KEY_REG: &str = "passkey_reg";
const SESSION_KEY_AUTH: &str = "passkey_auth";

/// Browser options produced when passkey registration begins.
#[derive(Debug)]
pub struct PasskeyRegistrationChallenge {
    /// Base64url-encoded challenge.
    pub challenge: String,
    /// Account email bound to the ceremony.
    pub user_email: String,
    /// Relying-party identifier.
    pub rp_id: String,
    /// Full browser credential-creation options.
    pub raw_options: CreationChallengeResponse,
}

/// Browser options produced when passkey authentication begins.
#[derive(Debug)]
pub struct PasskeyAuthenticationChallenge {
    /// Base64url-encoded challenge.
    pub challenge: String,
    /// Account email bound to the ceremony.
    pub user_email: String,
    /// Full browser credential-request options.
    pub raw_options: RequestChallengeResponse,
}

fn require_session_present(operation: &'static str) -> Result<(), FrameworkError> {
    if session_mut(|_| ()).is_some() {
        return Ok(());
    }
    Err(FrameworkError::internal(format!(
        "{operation} requires SessionMiddleware so the ceremony selector survives the round trip"
    )))
}

fn store_selector(key: &str, selector: String) -> Result<(), FrameworkError> {
    require_session_present("passkey ceremony")?;
    session_mut(|session| session.put(key, selector));
    Ok(())
}

fn take_selector(key: &str, message: &str) -> Result<String, FrameworkError> {
    let selector = session()
        .and_then(|session| session.get::<String>(key))
        .ok_or_else(|| FrameworkError::Domain {
            message: message.to_owned(),
            status_code: 400,
        })?;
    session_mut(|session| session.forget(key));
    Ok(selector)
}

fn map_error(error: magnetar::Error) -> FrameworkError {
    match error {
        magnetar::Error::InvalidInput { field, message } => FrameworkError::Domain {
            status_code: match field.as_str() {
                "actor" => 401,
                "reauth" => 403,
                "credential" | "credentials" => 401,
                _ => 400,
            },
            message,
        },
        magnetar::Error::NotFound { resource, .. } if resource == "credential actor" => {
            FrameworkError::Domain {
                message: "passkey authentication failed".to_owned(),
                status_code: 401,
            }
        }
        magnetar::Error::Conflict { message, .. }
        | magnetar::Error::NotFound {
            identifier: message,
            ..
        } => FrameworkError::Domain {
            message,
            status_code: 400,
        },
        error => FrameworkError::internal(format!("Magnetar passkey operation: {error}")),
    }
}

/// Passkey authentication facade returned by `Auth::passkey`.
pub struct PasskeyAuth;

impl PasskeyAuth {
    /// Begin a passkey registration ceremony.
    ///
    /// # Errors
    ///
    /// Returns an error when session middleware or the passkey engine is not
    /// installed, owner/reauth policy rejects enrollment, or storage fails.
    pub async fn begin_registration(
        &self,
        email: &str,
    ) -> Result<PasskeyRegistrationChallenge, FrameworkError> {
        require_session_present("passkey registration")?;
        let engine = super::passkey_engine()?;
        let begun = engine
            .passkey_begin_registration(magnetar::passkey::RegistrationIntent {
                email: email.to_owned(),
                // The retained facade's legacy data session does not carry
                // Magnetar's opaque session id and auth epoch. Never promote
                // its bare user id into a credential actor; existing-account
                // enrollment must use the verified RequestContext plugin path.
                actor: None,
                reauthenticated_at: session()
                    .and_then(|session| session.password_confirmed_at())
                    .and_then(|timestamp| {
                        chrono::DateTime::<chrono::Utc>::from_timestamp(timestamp, 0)
                    }),
            })
            .await
            .map_err(map_error)?;
        let challenge = URL_SAFE_NO_PAD.encode(&*begun.options.public_key.challenge);
        let rp_id = begun.options.public_key.rp.id.clone();
        store_selector(SESSION_KEY_REG, begun.selector)?;
        Ok(PasskeyRegistrationChallenge {
            challenge,
            user_email: email.to_owned(),
            rp_id,
            raw_options: begun.options,
        })
    }

    /// Complete a passkey registration ceremony.
    ///
    /// # Errors
    ///
    /// Returns an error when the ceremony is missing, consumed, mismatched, or
    /// the credential fails WebAuthn verification.
    pub async fn finish_registration(
        &self,
        email: &str,
        response: RegisterPublicKeyCredential,
    ) -> Result<User, FrameworkError> {
        let engine = super::passkey_engine()?;
        let selector = take_selector(
            SESSION_KEY_REG,
            "passkey registration not started or expired",
        )?;
        engine
            .passkey_finish_registration(&selector, email, &response)
            .await
            .map_err(map_error)
    }

    /// Begin a passkey authentication ceremony.
    ///
    /// # Errors
    ///
    /// Returns an error when session middleware or the passkey engine is not
    /// installed, the account has no credentials, or storage fails.
    pub async fn begin_authentication(
        &self,
        email: &str,
    ) -> Result<PasskeyAuthenticationChallenge, FrameworkError> {
        require_session_present("passkey authentication")?;
        let engine = super::passkey_engine()?;
        let begun = engine
            .passkey_begin_authentication(email)
            .await
            .map_err(map_error)?;
        let challenge = URL_SAFE_NO_PAD.encode(&*begun.options.public_key.challenge);
        store_selector(SESSION_KEY_AUTH, begun.selector)?;
        Ok(PasskeyAuthenticationChallenge {
            challenge,
            user_email: email.to_owned(),
            raw_options: begun.options,
        })
    }

    /// Complete a passkey authentication ceremony and issue a session.
    ///
    /// # Errors
    ///
    /// Returns an error when the ceremony or credential is invalid, a second
    /// factor is required, or session issuance fails.
    pub async fn finish_authentication(
        &self,
        email: &str,
        response: PublicKeyCredential,
    ) -> Result<(User, Session), FrameworkError> {
        let engine = super::passkey_engine()?;
        let selector = take_selector(
            SESSION_KEY_AUTH,
            "passkey authentication not started or expired",
        )?;
        let decision = engine
            .passkey_finish_authentication(
                &selector,
                email,
                &response,
                magnetar::sessions::SessionMetadata::default(),
            )
            .await
            .map_err(map_error)?;
        let issued = match decision {
            super::engine::HostSignInDecision::SessionAllowed(issued) => issued,
            super::engine::HostSignInDecision::FactorRequired { .. } => {
                return Err(FrameworkError::Domain {
                    message: "second-factor authentication is required".to_owned(),
                    status_code: 401,
                });
            }
        };
        super::bind_issued_session(&issued, false);
        let user = engine
            .passkey_user_by_id(issued.session.user_id.as_str())
            .await
            .map_err(map_error)?;
        Ok((user, issued.session))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_actor_maps_to_generic_passkey_authentication_failure() {
        let error = map_error(magnetar::Error::NotFound {
            resource: "credential actor".to_owned(),
            identifier: "expired or revoked".to_owned(),
        });

        assert!(matches!(
            error,
            FrameworkError::Domain {
                message,
                status_code: 401,
            } if message == "passkey authentication failed"
        ));
    }
}
