//! Password authentication through the installed Magnetar engine.

use super::{Session, SignInOutcome, User};
use crate::error::FrameworkError;

/// Password authentication facade returned by `Auth::password`.
pub struct PasswordAuth;

impl PasswordAuth {
    /// Register a password credential without exposing whether the email exists.
    ///
    /// # Errors
    ///
    /// Returns an error when the engine is not installed or registration fails.
    pub async fn register(&self, email: &str, password: &str) -> Result<User, FrameworkError> {
        super::abuse_limiter::check_auth_abuse(
            super::abuse_limiter::AuthAbuseRoute::PasswordRegister,
            email,
        )
        .await?;
        let engine = super::password_engine()?;
        engine
            .password_register(magnetar::plugins::password::RegisterInput {
                email: email.to_owned(),
                password: secrecy::SecretString::from(password.to_owned()),
            })
            .await
            .map_err(map_magnetar_password_error)
    }

    /// Verify a password, run the factor gate, and return a fresh session.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid credentials, lockout, a required second
    /// factor, missing engine initialization, or storage failure.
    pub async fn authenticate(
        &self,
        email: &str,
        password: &str,
        user_agent: Option<String>,
        ip_address: Option<String>,
    ) -> Result<(User, Session), FrameworkError> {
        self.authenticate_outcome(email, password, user_agent, ip_address)
            .await?
            .into_legacy_tuple("second-factor authentication is required")
    }

    /// Verify a password and preserve any required factor continuation.
    ///
    /// A [`SignInOutcome::FactorRequired`] result does not bind the framework
    /// session. Its selector can be completed through the retained host engine.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid credentials, lockout, missing engine
    /// initialization, or storage failure.
    pub async fn authenticate_outcome(
        &self,
        email: &str,
        password: &str,
        user_agent: Option<String>,
        ip_address: Option<String>,
    ) -> Result<SignInOutcome, FrameworkError> {
        super::bind_scope_preflight()?;
        let engine = super::password_engine()?;
        let (user, decision) = engine
            .password_sign_in(magnetar::plugins::password::PasswordAttempt {
                email: email.to_owned(),
                password: secrecy::SecretString::from(password.to_owned()),
                metadata: magnetar::sessions::SessionMetadata {
                    user_agent,
                    ip_address,
                },
            })
            .await
            .map_err(map_magnetar_password_error)?;
        match decision {
            super::engine::HostSignInDecision::SessionAllowed(issued) => {
                super::bind_issued_session(&issued, true)?;
                Ok(SignInOutcome::Authenticated {
                    user,
                    session: issued.session,
                })
            }
            super::engine::HostSignInDecision::FactorRequired { challenge_selector } => {
                Ok(SignInOutcome::FactorRequired { challenge_selector })
            }
        }
    }
}

pub(super) fn map_magnetar_password_error(error: magnetar::Error) -> FrameworkError {
    match error {
        magnetar::Error::Conflict { message, .. }
        | magnetar::Error::NotFound {
            identifier: message,
            ..
        }
        | magnetar::Error::InvalidInput { message, .. } => FrameworkError::Domain {
            message,
            status_code: 401,
        },
        error => FrameworkError::internal(format!("Magnetar password operation: {error}")),
    }
}
