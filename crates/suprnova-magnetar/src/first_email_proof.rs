//! Atomic first-email-proof storage boundary.
//!
//! The host implementation owns proof consumption, provisional credential
//! cleanup, the verification compare-and-swap, and the proving mutation in one
//! transaction. Plugin services receive only committed user state and remain
//! the sole constructors of verified-principal witnesses.

use std::fmt;

use async_trait::async_trait;
use secrecy::SecretString;

use crate::Result;
use crate::storage::PresentedToken;

/// The mailbox-established mutation applied at the first proof boundary.
pub enum FirstEmailProofMutation {
    /// Consume a reset token and replace the account password.
    PasswordReset {
        /// The single-use password-reset token.
        token: PresentedToken,
        /// Optional caller expectation for the token owner.
        expected_user_id: Option<String>,
        /// The already-verified replacement password hash.
        new_password_hash: SecretString,
    },
    /// Consume a magic-link token and leave no durable primary credential.
    MagicLink {
        /// The single-use magic-link token.
        token: PresentedToken,
    },
    /// Consume an OAuth email-completion token and retain only its provider.
    OAuthEmailCompletion {
        /// The single-use OAuth email-completion token.
        token: PresentedToken,
    },
}

impl FirstEmailProofMutation {
    /// Return the non-secret kind of this proving mutation.
    #[must_use]
    pub const fn kind(&self) -> FirstEmailProofKind {
        match self {
            Self::PasswordReset { .. } => FirstEmailProofKind::PasswordReset,
            Self::MagicLink { .. } => FirstEmailProofKind::MagicLink,
            Self::OAuthEmailCompletion { .. } => FirstEmailProofKind::OAuthEmailCompletion,
        }
    }
}

impl fmt::Debug for FirstEmailProofMutation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct(match self.kind() {
                FirstEmailProofKind::PasswordReset => "PasswordReset",
                FirstEmailProofKind::MagicLink => "MagicLink",
                FirstEmailProofKind::OAuthEmailCompletion => "OAuthEmailCompletion",
            })
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

/// The mailbox proof flow that committed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FirstEmailProofKind {
    /// Password reset supplied the replacement credential.
    PasswordReset,
    /// Magic-link consumption proved mailbox control.
    MagicLink,
    /// OAuth email completion supplied the linked provider credential.
    OAuthEmailCompletion,
}

/// Neutral committed state returned by an atomic first-proof transition.
///
/// Storage adapters cannot construct [`crate::auth::VerifiedPrincipal`]. The
/// owning Magnetar service uses this committed state to construct that private
/// witness before invoking the factor gate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FirstEmailProofCommit {
    /// The application user whose proof committed.
    pub user_id: String,
    /// The proof flow that committed.
    pub kind: FirstEmailProofKind,
    /// Whether this caller won the verification compare-and-swap.
    pub first_proof: bool,
    /// Authentication epoch after the transaction.
    pub auth_epoch: u64,
    /// Provider account id retained by OAuth email completion.
    pub provider_account_id: Option<String>,
    /// Opaque sessions revoked by the transaction.
    pub revoked_sessions: u64,
    /// Remember-me rows revoked by the transaction.
    pub revoked_remember_rows: u64,
}

/// Result of applying one first-email-proof mutation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FirstEmailProofOutcome {
    /// The proving mutation committed.
    Committed(FirstEmailProofCommit),
    /// OAuth completion encountered an already verified email owner.
    ExplicitLinkRequired {
        /// Normalized email that collided with the verified account.
        normalized_email: String,
    },
}

impl FirstEmailProofOutcome {
    /// Extract a committed result for proof types that cannot return a link
    /// collision.
    pub fn into_commit(self) -> Result<FirstEmailProofCommit> {
        match self {
            Self::Committed(commit) => Ok(commit),
            Self::ExplicitLinkRequired { .. } => Err(crate::Error::Conflict {
                resource: "first-email-proof".to_owned(),
                message: "proof requires an explicit account-link flow".to_owned(),
            }),
        }
    }
}

/// Trusted provider identity used to initialize a verified account.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewVerifiedProviderAccount {
    /// Stable provider identifier.
    pub provider: String,
    /// Stable provider-side account identifier.
    pub provider_account_id: String,
    /// Provider-verified account email.
    pub email: String,
}

/// Neutral committed state returned after verified provider initialization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedProviderAccountCommit {
    /// The application user owning the provider identity.
    pub user_id: String,
    /// Initial or current authentication epoch.
    pub auth_epoch: u64,
}

/// Host-owned atomic first-email-proof persistence boundary.
#[async_trait]
pub trait FirstEmailProofStore: Send + Sync {
    /// Consume one proof and atomically apply its credential transition.
    async fn apply(&self, mutation: FirstEmailProofMutation) -> Result<FirstEmailProofOutcome>;

    /// Create a verified user and linked provider identity atomically.
    async fn create_verified_provider_account(
        &self,
        input: NewVerifiedProviderAccount,
    ) -> Result<VerifiedProviderAccountCommit>;
}
