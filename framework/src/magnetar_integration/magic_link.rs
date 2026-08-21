//! Magic-link authentication through the installed Magnetar engine.

use super::{Session, User};
use crate::error::FrameworkError;

/// Magic-link authentication facade returned by `Auth::magic_link`.
pub struct MagicLinkAuth;

impl MagicLinkAuth {
    /// Mint a single-use token for app-owned delivery.
    ///
    /// The callback URL remains part of the public facade because the caller
    /// owns mail composition; Magnetar returns only the plaintext token.
    ///
    /// # Errors
    ///
    /// Returns an error when rate limiting, engine initialization, or token
    /// storage fails.
    pub async fn send(&self, email: &str, _callback_url: &str) -> Result<String, FrameworkError> {
        super::abuse_limiter::check_auth_abuse(
            super::abuse_limiter::AuthAbuseRoute::MagicLinkSend,
            email,
        )
        .await?;
        let engine = super::password_engine()?;
        engine
            .magic_link_send(email)
            .await
            .map_err(map_magnetar_magic_link_error)
    }

    /// Consume a single-use token, run the factor gate, and return a session.
    ///
    /// # Errors
    ///
    /// Returns an error when the token is invalid or used, a second factor is
    /// required, the engine is not installed, or session issuance fails.
    pub async fn consume(&self, token: &str) -> Result<(User, Session), FrameworkError> {
        let engine = super::password_engine()?;
        let decision = engine
            .magic_link_consume(token, magnetar::sessions::SessionMetadata::default())
            .await
            .map_err(map_magnetar_magic_link_error)?;
        let issued = match decision {
            super::engine::HostSignInDecision::SessionAllowed(issued) => issued,
            super::engine::HostSignInDecision::FactorRequired { .. } => {
                return Err(FrameworkError::Domain {
                    message: "second-factor authentication is required".to_owned(),
                    status_code: 401,
                });
            }
        };
        let user = engine
            .user_by_id(issued.session.user_id.as_str())
            .await
            .map_err(map_magnetar_magic_link_error)?
            .ok_or_else(|| FrameworkError::internal("magic-link session user was not found"))?;
        Ok((user, issued.session))
    }
}

fn map_magnetar_magic_link_error(error: magnetar::Error) -> FrameworkError {
    match error {
        magnetar::Error::InvalidInput { message, .. }
        | magnetar::Error::Conflict { message, .. }
        | magnetar::Error::NotFound {
            identifier: message,
            ..
        } => FrameworkError::Domain {
            message,
            status_code: 401,
        },
        error => FrameworkError::internal(format!("Magnetar magic-link operation: {error}")),
    }
}
