//! Magic-link authentication through the installed Magnetar engine.

use super::{Session, SignInOutcome, User};
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
        self.consume_outcome(token)
            .await?
            .into_legacy_tuple("second-factor authentication is required")
    }

    /// Consume a single-use token and preserve any factor continuation.
    ///
    /// A [`SignInOutcome::FactorRequired`] result does not bind the framework
    /// session. Its selector can be completed through the retained host engine.
    ///
    /// # Errors
    ///
    /// Returns an error when the token is invalid or used, the engine is not
    /// installed, user lookup fails, or session issuance fails.
    pub async fn consume_outcome(&self, token: &str) -> Result<SignInOutcome, FrameworkError> {
        super::bind_scope_preflight()?;
        let session_authority = super::factor_engine()?;
        let engine = super::password_engine()?;
        let decision = engine
            .magic_link_consume(token, magnetar::sessions::SessionMetadata::default())
            .await
            .map_err(map_magnetar_magic_link_error)?;
        let issued = match decision {
            super::engine::HostSignInDecision::SessionAllowed(issued) => issued,
            super::engine::HostSignInDecision::FactorRequired { challenge_selector } => {
                return Ok(SignInOutcome::FactorRequired { challenge_selector });
            }
        };
        let user_id = issued.session.user_id.to_string();
        let (user, session) =
            super::handoff_issued_session(session_authority, *issued, false, async move {
                engine
                    .user_by_id(&user_id)
                    .await
                    .map_err(map_magnetar_magic_link_error)?
                    .ok_or_else(|| {
                        FrameworkError::internal("magic-link session user was not found")
                    })
            })
            .await?;
        Ok(SignInOutcome::Authenticated { user, session })
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
