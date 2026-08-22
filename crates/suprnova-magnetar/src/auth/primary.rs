//! Primary-authentication contracts shared by all credential providers.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::Result;
use crate::sessions::SessionMetadata;

/// The credential family that verified a primary sign-in.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SignInMethod {
    /// Password authentication.
    Password,
    /// Magic-link authentication.
    MagicLink,
    /// WebAuthn/passkey authentication.
    Passkey,
    /// An OAuth provider authentication.
    OAuth,
    /// A rotated, epoch-bound remember-me credential.
    Remembered,
    /// Device-approval authentication.
    DeviceApproval,
}

/// A provider-owned primary credential handed to [`PrimaryAuth::verify`].
///
/// The host does not interpret the credential payload. Each provider owns the
/// encoding and verification rules for its variant.
#[derive(Clone, Eq, PartialEq)]
pub enum PrimaryCredential {
    /// Password verifier input.
    Password(String),
    /// Magic-link token input.
    MagicLink(String),
    /// Passkey assertion bytes.
    Passkey(Vec<u8>),
    /// OAuth provider name and authorization response.
    OAuth {
        /// OAuth provider identifier.
        provider: String,
        /// Provider authorization response.
        response: String,
    },
    /// Device-approval token input.
    DeviceApproval(String),
}

impl std::fmt::Debug for PrimaryCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Password(_) => "Password",
            Self::MagicLink(_) => "MagicLink",
            Self::Passkey(_) => "Passkey",
            Self::OAuth { .. } => "OAuth",
            Self::DeviceApproval(_) => "DeviceApproval",
        };
        formatter.debug_tuple(name).field(&"[redacted]").finish()
    }
}

/// Context attached to a successful primary authentication.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AuthenticationContext {
    /// Session metadata supplied by the host.
    pub metadata: SessionMetadata,
    /// Authentication epoch observed while verifying the principal.
    pub auth_epoch: u64,
    /// Time at which the primary credential was verified.
    pub authenticated_at: DateTime<Utc>,
}

impl AuthenticationContext {
    /// Construct authentication context from host metadata and an epoch.
    #[must_use]
    pub const fn new(
        metadata: SessionMetadata,
        auth_epoch: u64,
        authenticated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            metadata,
            auth_epoch,
            authenticated_at,
        }
    }
}

/// A principal whose primary credential has already been verified.
///
/// Fields are intentionally private. Only code inside the primary-auth
/// boundary can create this witness; callers can inspect it and pass it to the
/// shared factor gate but cannot forge a verified principal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedPrincipal {
    user_id: String,
    method: SignInMethod,
    metadata: AuthenticationContext,
}

impl VerifiedPrincipal {
    /// Return the application user identifier.
    #[must_use]
    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    /// Return the primary authentication method.
    #[must_use]
    pub fn method(&self) -> &SignInMethod {
        &self.method
    }

    /// Return the verified authentication context.
    #[must_use]
    pub fn context(&self) -> &AuthenticationContext {
        &self.metadata
    }

    /// Construct a principal at the primary-auth boundary.
    #[allow(dead_code)]
    pub(crate) fn new(
        user_id: String,
        method: SignInMethod,
        metadata: AuthenticationContext,
    ) -> Result<Self> {
        if user_id.is_empty() {
            return Err(crate::Error::InvalidInput {
                field: "user_id".to_owned(),
                message: "must not be empty".to_owned(),
            });
        }
        Ok(Self {
            user_id,
            method,
            metadata,
        })
    }
}

/// The private witness produced after a successful second-factor proof.
///
/// This type has no public constructor. It is deliberately kept separate from
/// [`VerifiedPrincipal`] so a primary credential cannot be mistaken for a
/// completed factor ceremony.
#[derive(Debug)]
pub(crate) struct FactorGateApproval {
    pub(crate) user_id: String,
    pub(crate) context: AuthenticationContext,
}

/// A provider-specific primary authentication verifier.
#[async_trait]
pub trait PrimaryAuth: Send + Sync {
    /// Verify a primary credential and return a host-owned principal witness.
    async fn verify(&self, input: PrimaryCredential) -> Result<VerifiedPrincipal>;
}
