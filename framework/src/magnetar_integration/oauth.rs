//! OAuth authentication through the installed Magnetar provider engine.

use secrecy::SecretString;
use sha2::{Digest, Sha256};

use super::{Session, SignInOutcome, User};
use crate::error::FrameworkError;
use crate::session::session;

/// Result of initiating an OAuth authorization flow.
#[derive(Debug)]
pub struct OAuthKickoff {
    /// Provider authorization URL.
    pub authorization_url: String,
    /// Single-use state selector bound to the initiating session.
    pub state: String,
}

/// Verified provider identity without account or session side effects.
#[derive(Debug, Clone)]
pub struct OAuthIdentity {
    /// Provider registry name.
    pub provider: String,
    /// Stable provider subject.
    pub subject: String,
    /// Verified provider email, when supplied.
    pub email: Option<String>,
    /// Provider display name, when supplied.
    pub name: Option<String>,
    /// URL of the account's picture as the provider reported it, or `None`
    /// when the profile has no picture or an empty one.
    ///
    /// Untrusted profile data: the provider account's owner controls it.
    /// Check its scheme (`https`) and length before you render, fetch or
    /// store it.
    pub avatar_url: Option<String>,
}

/// Verified Apple identity returned by [`OAuthAuth::verify_apple_identity`].
#[derive(Debug, Clone)]
pub struct AppleIdentity {
    /// Provider registry name (`apple`).
    pub provider: String,
    /// Stable Apple subject.
    pub subject: String,
    /// Verified email, when Apple supplied it.
    pub email: Option<String>,
    /// Whether an email was supplied and verified.
    pub email_verified: bool,
    /// Whether the email is an Apple private relay address.
    pub is_private_email: bool,
}

/// OAuth facade returned by [`crate::Auth::oauth`].
pub struct OAuthAuth {
    provider: String,
}

impl OAuthAuth {
    pub(crate) fn new(provider: String) -> Self {
        Self { provider }
    }

    /// Begin a session-bound sign-in flow.
    ///
    /// The flow is bound to the current session, so this also marks the
    /// session for storage: `SessionMiddleware` then sends its cookie with
    /// the response, even when the start is the browser's first request.
    ///
    /// # Errors
    ///
    /// Returns an error when session middleware or the OAuth engine/provider is
    /// not configured, rate limiting rejects the request, or ceremony storage fails.
    pub async fn begin(&self) -> Result<OAuthKickoff, FrameworkError> {
        let session_id = session()
            .map(|session| session.id)
            .filter(|session_id| !session_id.is_empty())
            .ok_or_else(|| FrameworkError::internal("OAuth begin requires SessionMiddleware"))?;
        let engine = oauth_engine(&self.provider)?;
        let digest: [u8; 32] = Sha256::digest(session_id.as_bytes()).into();
        let begun = engine
            .oauth_begin(super::engine::MagnetarOAuthBegin {
                provider: self.provider.clone(),
                intent: magnetar::oauth::authorization::OAuthIntent::SignIn,
                actor: None,
                binding: magnetar::oauth::authorization::CeremonyBinding::HostSessionDigest(digest),
                limiter_identity: format!("{}:{session_id}", self.provider),
            })
            .await
            .map_err(map_error)?;
        // The ceremony is bound to this session, so the session must reach
        // the browser. A first-contact start that changed nothing else, such
        // as a JSON or POST request, would otherwise be neither stored nor
        // sent as a cookie, and the callback would arrive under another
        // session and fail the binding.
        crate::session::session_mut(|session| session.dirty = true);
        Ok(OAuthKickoff {
            authorization_url: begun.authorization_url,
            state: begun.state,
        })
    }

    /// Verify a callback identity without account or session completion.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid/consumed state, provider proof failure,
    /// missing session binding, or an unconfigured provider.
    pub async fn verify_oauth_identity(
        &self,
        code: &str,
        state: &str,
    ) -> Result<OAuthIdentity, FrameworkError> {
        let (identity, avatar_url) = self.verify_identity(code, state, None).await?;
        Ok(OAuthIdentity {
            provider: identity.provider,
            subject: identity.subject,
            email: identity.email,
            name: identity.display_name,
            avatar_url,
        })
    }

    /// Verify an Apple callback identity, including `form_post` user data.
    ///
    /// # Errors
    ///
    /// Returns an error when the provider is not Apple or callback proof fails.
    pub async fn verify_apple_identity(
        &self,
        code: &str,
        state: &str,
        form_post_user: Option<String>,
    ) -> Result<AppleIdentity, FrameworkError> {
        let (identity, _) = self.verify_identity(code, state, form_post_user).await?;
        if identity.provider != "apple" {
            return Err(FrameworkError::Domain {
                message: "configured provider is not Apple".to_owned(),
                status_code: 400,
            });
        }
        let email_verified = identity.email.is_some() && identity.email_verified;
        let is_private_email = identity
            .email
            .as_deref()
            .is_some_and(|email| email.ends_with("@privaterelay.appleid.com"));
        Ok(AppleIdentity {
            provider: identity.provider,
            subject: identity.subject,
            email: identity.email,
            email_verified,
            is_private_email,
        })
    }

    /// Complete an Apple `form_post` callback and issue a session.
    ///
    /// # Errors
    ///
    /// Returns an error for callback proof, identity policy, factor-gate, or
    /// session issuance failure.
    pub async fn complete_with_apple_form_post(
        &self,
        code: &str,
        state: &str,
        form_post_user: Option<String>,
    ) -> Result<(User, Session), FrameworkError> {
        self.complete_with_apple_form_post_outcome(code, state, form_post_user)
            .await?
            .into_legacy_tuple("OAuth sign-in requires a second factor")
    }

    /// Complete an Apple `form_post` callback and preserve a factor continuation.
    ///
    /// A [`SignInOutcome::FactorRequired`] result does not bind the framework
    /// session. Its selector can be completed through the retained host engine.
    ///
    /// # Errors
    ///
    /// Returns an error for callback proof, identity policy, or session
    /// issuance failure.
    pub async fn complete_with_apple_form_post_outcome(
        &self,
        code: &str,
        state: &str,
        form_post_user: Option<String>,
    ) -> Result<SignInOutcome, FrameworkError> {
        self.complete_callback_outcome(code, state, form_post_user)
            .await
    }

    /// Complete a standard OAuth callback and issue a session.
    ///
    /// # Errors
    ///
    /// Returns an error for callback proof, identity policy, factor-gate, or
    /// session issuance failure.
    pub async fn complete(
        &self,
        code: &str,
        state: &str,
    ) -> Result<(User, Session), FrameworkError> {
        self.complete_outcome(code, state)
            .await?
            .into_legacy_tuple("OAuth sign-in requires a second factor")
    }

    /// Complete a standard OAuth callback and preserve a factor continuation.
    ///
    /// A [`SignInOutcome::FactorRequired`] result does not bind the framework
    /// session. Its selector can be completed through the retained host engine.
    ///
    /// # Errors
    ///
    /// Returns an error for callback proof, identity policy, or session
    /// issuance failure.
    pub async fn complete_outcome(
        &self,
        code: &str,
        state: &str,
    ) -> Result<SignInOutcome, FrameworkError> {
        self.complete_callback_outcome(code, state, None).await
    }

    /// The verified identity and the account picture URL the provider
    /// reported with it.
    async fn verify_identity(
        &self,
        code: &str,
        state: &str,
        form_post_user: Option<String>,
    ) -> Result<
        (
            magnetar::oauth::identity::VerifiedProviderIdentity,
            Option<String>,
        ),
        FrameworkError,
    > {
        let engine = oauth_engine(&self.provider)?;
        engine
            .oauth_verify_identity_with_avatar(super::engine::MagnetarOAuthCallback {
                provider: self.provider.clone(),
                state: state.to_owned(),
                code: SecretString::from(code.to_owned()),
                host_session_digest: session_digest(),
                form_post_user,
                metadata: magnetar::sessions::SessionMetadata::default(),
            })
            .await
            .map_err(map_error)
    }

    async fn complete_callback_outcome(
        &self,
        code: &str,
        state: &str,
        form_post_user: Option<String>,
    ) -> Result<SignInOutcome, FrameworkError> {
        let session_authority = super::factor_engine()?;
        super::factor_bind_scope_preflight()?;
        let engine = oauth_engine(&self.provider)?;
        match engine
            .oauth_complete(super::engine::MagnetarOAuthCallback {
                provider: self.provider.clone(),
                state: state.to_owned(),
                code: SecretString::from(code.to_owned()),
                host_session_digest: session_digest(),
                form_post_user,
                metadata: magnetar::sessions::SessionMetadata::default(),
            })
            .await
            .map_err(map_error)?
        {
            super::engine::MagnetarOAuthCompletion::SessionAllowed { user, session } => {
                let (user, session) =
                    super::handoff_issued_session(session_authority, *session, false, async move {
                        Ok(user)
                    })
                    .await?;
                Ok(SignInOutcome::Authenticated { user, session })
            }
            super::engine::MagnetarOAuthCompletion::FactorRequired { challenge_selector } => {
                Ok(SignInOutcome::FactorRequired { challenge_selector })
            }
            super::engine::MagnetarOAuthCompletion::EmailCompletionRequired { .. } => {
                Err(FrameworkError::Domain {
                    message: "OAuth identity requires verified email completion".to_owned(),
                    status_code: 409,
                })
            }
            super::engine::MagnetarOAuthCompletion::ExplicitLinkRequired { .. } => {
                Err(FrameworkError::Domain {
                    message: "OAuth identity must be linked explicitly".to_owned(),
                    status_code: 409,
                })
            }
            super::engine::MagnetarOAuthCompletion::AccountCreated { .. }
            | super::engine::MagnetarOAuthCompletion::AccountLinked { .. } => Err(
                FrameworkError::internal("OAuth callback completed without a sign-in session"),
            ),
        }
    }
}

fn oauth_engine(
    provider: &str,
) -> Result<&'static std::sync::Arc<dyn super::engine::MagnetarOAuthAuthEngine>, FrameworkError> {
    let guard = super::engine_install_guard()?;
    super::ensure_engine_installation_ready(guard.reserved)?;
    let engine = super::MAGNETAR_OAUTH_ENGINE
        .get()
        .ok_or_else(|| {
            FrameworkError::internal(
                "Magnetar OAuth authentication subsystem was not initialized during application bootstrap; configure MagnetarConfig::oauth(...) before init_magnetar(...), or use init_magnetar_oauth_only(...), install_magnetar_oauth_engine(...), or install_magnetar_oauth_engine_with_factor(...)",
            )
        })?;
    if !engine.oauth_supports_provider(provider) {
        return Err(provider_not_configured(provider));
    }
    Ok(engine)
}

fn session_digest() -> Option<[u8; 32]> {
    session()
        .map(|session| session.id)
        .filter(|session_id| !session_id.is_empty())
        .map(|session_id| Sha256::digest(session_id.as_bytes()).into())
}

fn provider_not_configured(provider: &str) -> FrameworkError {
    FrameworkError::Domain {
        message: format!("OAuth provider '{provider}' is not configured"),
        status_code: 400,
    }
}

/// Every arm builds its error through a constructor, not a struct
/// literal, so a 5xx records its frames for the development error page.
fn map_error(error: super::engine::HostOAuthError) -> FrameworkError {
    match error {
        super::engine::HostOAuthError::Protocol(error) => {
            FrameworkError::domain(error.to_string(), error.class().status())
        }
        super::engine::HostOAuthError::Auth(magnetar::Error::InvalidInput { message, .. })
        | super::engine::HostOAuthError::Auth(magnetar::Error::NotFound {
            identifier: message,
            ..
        })
        | super::engine::HostOAuthError::Auth(magnetar::Error::Conflict { message, .. }) => {
            FrameworkError::domain(message, 400)
        }
        super::engine::HostOAuthError::Auth(magnetar::Error::DependencyUnavailable { .. }) => {
            FrameworkError::domain("OAuth dependency unavailable", 502)
        }
        super::engine::HostOAuthError::Auth(error) => {
            FrameworkError::internal(format!("Magnetar OAuth operation: {error}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorReport;
    use crate::error::frames::record_frames;

    #[tokio::test]
    async fn an_unavailable_oauth_dependency_is_a_502_that_records_its_frames() {
        let (status, recorded) = record_frames(true, async {
            let error = map_error(super::super::engine::HostOAuthError::Auth(
                magnetar::Error::DependencyUnavailable {
                    dependency: "github".to_owned(),
                    message: "connect timeout".to_owned(),
                },
            ));
            (
                error.status_code(),
                ErrorReport::from_error(&error).frames().is_some(),
            )
        })
        .await;

        assert_eq!(status, 502);
        assert!(
            recorded,
            "a 5xx the framework builds must record its frames for the development error page"
        );
    }
}
