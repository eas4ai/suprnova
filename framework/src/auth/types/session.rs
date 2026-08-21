//! Authentication session values exposed by Suprnova.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::error::FrameworkError;

use super::{SessionToken, UserId};

/// An authentication session with a plaintext token only when it was freshly issued.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Session {
    /// The plaintext credential for a freshly created session.
    ///
    /// Sessions loaded from persistent storage contain only [`Session::token_hash`]
    /// and set this field to `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<SessionToken>,
    /// The lowercase hexadecimal SHA-256 digest used for persistent lookup.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token_hash: String,
    /// The opaque account identifier bound to this session.
    pub user_id: UserId,
    /// The optional client user-agent string captured when the session was created.
    pub user_agent: Option<String>,
    /// The optional client IP address captured when the session was created.
    pub ip_address: Option<String>,
    /// When the session was created.
    pub created_at: DateTime<Utc>,
    /// When the session was last updated.
    pub updated_at: DateTime<Utc>,
    /// When the session expires.
    pub expires_at: DateTime<Utc>,
}

impl Session {
    /// Starts constructing a session with legacy-compatible defaults.
    #[must_use]
    pub fn builder() -> SessionBuilder {
        SessionBuilder::default()
    }

    /// Returns whether the session has passed its expiry timestamp.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }
}

/// Builder for a [`Session`].
#[derive(Default)]
pub struct SessionBuilder {
    token: Option<SessionToken>,
    token_hash: Option<String>,
    user_id: Option<UserId>,
    user_agent: Option<String>,
    ip_address: Option<String>,
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
}

impl SessionBuilder {
    /// Sets a newly issued plaintext token.
    ///
    /// Its digest always determines the resulting [`Session::token_hash`].
    #[must_use]
    pub fn token(mut self, token: SessionToken) -> Self {
        self.token = Some(token);
        self
    }

    /// Sets a digest for a session loaded without its plaintext token.
    #[must_use]
    pub fn token_hash(mut self, token_hash: String) -> Self {
        self.token_hash = Some(token_hash);
        self
    }

    /// Sets the required user identifier.
    #[must_use]
    pub fn user_id(mut self, user_id: UserId) -> Self {
        self.user_id = Some(user_id);
        self
    }

    /// Sets the optional client user-agent string.
    #[must_use]
    pub fn user_agent(mut self, user_agent: Option<String>) -> Self {
        self.user_agent = user_agent;
        self
    }

    /// Sets the optional client IP address.
    #[must_use]
    pub fn ip_address(mut self, ip_address: Option<String>) -> Self {
        self.ip_address = ip_address;
        self
    }

    /// Sets the creation timestamp.
    #[must_use]
    pub fn created_at(mut self, created_at: DateTime<Utc>) -> Self {
        self.created_at = Some(created_at);
        self
    }

    /// Sets the last-update timestamp.
    #[must_use]
    pub fn updated_at(mut self, updated_at: DateTime<Utc>) -> Self {
        self.updated_at = Some(updated_at);
        self
    }

    /// Sets the expiry timestamp.
    #[must_use]
    pub fn expires_at(mut self, expires_at: DateTime<Utc>) -> Self {
        self.expires_at = Some(expires_at);
        self
    }

    /// Builds a session, generating a token and default timestamps when absent.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError`] when no user identifier was supplied.
    pub fn build(self) -> Result<Session, FrameworkError> {
        let now = Utc::now();
        let user_id = self
            .user_id
            .ok_or_else(|| FrameworkError::bad_request("a user identifier is required"))?;

        let (token, token_hash) = match (self.token, self.token_hash) {
            (Some(token), _) => {
                let token_hash = token.token_hash();
                (Some(token), token_hash)
            }
            (None, Some(token_hash)) => (None, token_hash),
            (None, None) => {
                let token = SessionToken::new_random();
                let token_hash = token.token_hash();
                (Some(token), token_hash)
            }
        };

        Ok(Session {
            token,
            token_hash,
            user_id,
            user_agent: self.user_agent,
            ip_address: self.ip_address,
            created_at: self.created_at.unwrap_or(now),
            updated_at: self.updated_at.unwrap_or(now),
            expires_at: self.expires_at.unwrap_or(now + Duration::days(30)),
        })
    }
}
