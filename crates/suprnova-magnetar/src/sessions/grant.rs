//! Safe conversion of one issued session into host carriers.

use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};

use super::{SessionMetadata, invalid};

/// A non-bearer reference suitable for an authenticated host data session.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WebSessionBinding {
    /// Generated session identifier.
    pub session_id: String,
    /// Digest generated at issuance and persisted with the session row.
    pub token_digest: [u8; 32],
}

/// A bearer carrier that can expose its token exactly once by consumption.
#[derive(Debug)]
pub struct BearerSession {
    session_id: String,
    user_id: String,
    token: SecretString,
    expires_at: DateTime<Utc>,
    metadata: SessionMetadata,
}

impl BearerSession {
    /// Return the generated session identifier.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    /// Return the application user identifier.
    pub fn user_id(&self) -> &str {
        &self.user_id
    }
    /// Return the session expiry.
    pub fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }
    /// Return issuance metadata.
    pub fn metadata(&self) -> &SessionMetadata {
        &self.metadata
    }
    /// Consume the carrier and move the plaintext token to its host adapter.
    pub fn expose_token_once(self) -> SecretString {
        self.token
    }
}

#[cfg(feature = "device-authorization")]
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct SessionGrantSnapshot {
    session_id: String,
    user_id: String,
    opaque_token: String,
    expires_at: DateTime<Utc>,
    metadata: SessionMetadata,
}

/// The result of a successful session issuance.
///
/// All security-sensitive fields are private. The bearer is never serialized
/// as part of a web binding and can only be moved by consuming this grant.
#[derive(Debug)]
pub struct SessionGrant {
    session_id: String,
    user_id: String,
    opaque_token: SecretString,
    token_digest: [u8; 32],
    expires_at: DateTime<Utc>,
    metadata: SessionMetadata,
}

impl SessionGrant {
    /// Construct a grant inside the session issuance boundary.
    #[allow(dead_code)]
    pub(crate) fn new(
        session_id: String,
        user_id: String,
        opaque_token: SecretString,
        expires_at: DateTime<Utc>,
        metadata: SessionMetadata,
    ) -> Result<Self, crate::Error> {
        Self::new_at(
            session_id,
            user_id,
            opaque_token,
            expires_at,
            metadata,
            Utc::now(),
        )
    }

    #[allow(dead_code)]
    pub(crate) fn new_at(
        session_id: String,
        user_id: String,
        opaque_token: SecretString,
        expires_at: DateTime<Utc>,
        metadata: SessionMetadata,
        now: DateTime<Utc>,
    ) -> Result<Self, crate::Error> {
        if session_id.is_empty() {
            return Err(invalid("session_id", "must not be empty"));
        }
        if user_id.is_empty() {
            return Err(invalid("user_id", "must not be empty"));
        }
        if expires_at <= now {
            return Err(invalid("expires_at", "must be in the future"));
        }
        let token_digest = digest_secret(&opaque_token);
        Ok(Self {
            session_id,
            user_id,
            opaque_token,
            token_digest,
            expires_at,
            metadata,
        })
    }

    /// Return the generated session identifier.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    /// Return the application user identifier.
    pub fn user_id(&self) -> &str {
        &self.user_id
    }
    /// Return the session expiry.
    pub fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }
    /// Return issuance metadata.
    pub fn metadata(&self) -> &SessionMetadata {
        &self.metadata
    }
    /// Build the digest-based web reference without exposing or re-hashing a bearer.
    pub fn web_binding(&self) -> WebSessionBinding {
        WebSessionBinding {
            session_id: self.session_id.clone(),
            token_digest: self.token_digest,
        }
    }
    /// Consume this grant and move its bearer token into an API carrier.
    pub fn into_bearer(self) -> BearerSession {
        BearerSession {
            session_id: self.session_id,
            user_id: self.user_id,
            token: self.opaque_token,
            expires_at: self.expires_at,
            metadata: self.metadata,
        }
    }
    #[cfg(feature = "device-authorization")]
    pub(crate) fn into_snapshot(self) -> SessionGrantSnapshot {
        SessionGrantSnapshot {
            session_id: self.session_id,
            user_id: self.user_id,
            opaque_token: self.opaque_token.expose_secret().to_owned(),
            expires_at: self.expires_at,
            metadata: self.metadata,
        }
    }

    #[cfg(feature = "device-authorization")]
    pub(crate) fn from_snapshot(snapshot: SessionGrantSnapshot) -> Result<Self, crate::Error> {
        Self::new(
            snapshot.session_id,
            snapshot.user_id,
            SecretString::from(snapshot.opaque_token),
            snapshot.expires_at,
            snapshot.metadata,
        )
    }

    /// Internal digest used by persistence adapters at issuance.
    #[allow(dead_code)]
    pub(crate) fn token_digest(&self) -> [u8; 32] {
        self.token_digest
    }
}

#[allow(dead_code)]
pub(crate) fn digest_secret(value: &SecretString) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(value.expose_secret().as_bytes());
    hasher.finalize().into()
}

pub(crate) fn digest_token(value: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hasher.finalize().into()
}
