//! Lossless application-binding records read from supported legacy schemas.

use secrecy::SecretString;

/// One application user imported from a legacy authentication schema.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedUser {
    /// Stable source identifier rendered without lossy conversion.
    pub source_user_id: String,
    /// Existing application identifier that must be retained, when known.
    pub preferred_app_user_id: Option<i64>,
    /// Source email preserved verbatim; matching uses normalized copies only.
    pub email: String,
    /// Optional application-owned display name.
    pub name: Option<String>,
    /// Opaque password hash, including an intentionally empty sentinel.
    pub password_hash: Option<String>,
    /// Verification timestamp preserved in the source database representation.
    pub email_verified_at: Option<String>,
    /// Lockout timestamp preserved in the source database representation.
    pub locked_at: Option<String>,
    /// Creation timestamp preserved in the source database representation.
    pub created_at: Option<String>,
    /// Update timestamp preserved in the source database representation.
    pub updated_at: Option<String>,
    /// Existing global revocation epoch, when the source exposes one.
    pub auth_epoch: Option<i64>,
    /// Existing session revocation version, when the source exposes one.
    pub session_version: Option<i64>,
}

/// One linked OAuth account with its source timestamps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedLinkedAccount {
    /// Resolved application user owner.
    pub app_user_id: i64,
    /// Provider key.
    pub provider: String,
    /// Provider-owned stable subject.
    pub subject: String,
    /// Source creation timestamp.
    pub created_at: Option<String>,
    /// Source update timestamp.
    pub updated_at: Option<String>,
}

/// One durable verification or password-reset token.
#[derive(Clone, Debug)]
pub struct ImportedSecureToken {
    /// Resolved application user owner.
    pub app_user_id: i64,
    /// Opaque source token. Debug output remains redacted by `SecretString`.
    pub token: SecretString,
    /// Source token purpose.
    pub purpose: String,
    /// Source consumed timestamp.
    pub used_at: Option<String>,
    /// Source expiry timestamp.
    pub expires_at: String,
    /// Source creation timestamp.
    pub created_at: String,
    /// Source update timestamp.
    pub updated_at: String,
}

/// One durable failed-login observation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedFailedLoginAttempt {
    /// Stable source row identifier for lossless idempotency.
    pub source_record_id: String,
    /// Email subject preserved verbatim.
    pub email: String,
    /// Optional source network address.
    pub ip_address: Option<String>,
    /// Attempt timestamp.
    pub attempted_at: String,
}

/// One encrypted two-factor enrollment and replay state.
#[derive(Clone, Debug)]
pub struct ImportedTwoFactorCredential {
    /// Resolved application user owner.
    pub app_user_id: i64,
    /// Existing encrypted TOTP secret ciphertext.
    pub secret: SecretString,
    /// Enrollment confirmation timestamp.
    pub confirmed_at: Option<String>,
    /// Existing encrypted recovery-code ciphertext.
    pub recovery_codes: Option<SecretString>,
    /// Last accepted TOTP timestep.
    pub last_used_timestep: Option<i64>,
    /// Source creation timestamp.
    pub created_at: String,
    /// Source update timestamp.
    pub updated_at: String,
}

/// A durable non-user authentication record sent to the host transaction.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum DurableAuthRecord {
    /// Linked OAuth account.
    LinkedAccount(ImportedLinkedAccount),
    /// Verification or password-reset token.
    SecureToken(ImportedSecureToken),
    /// Failed-login observation.
    FailedLoginAttempt(ImportedFailedLoginAttempt),
    /// Encrypted two-factor enrollment.
    TwoFactorCredential(ImportedTwoFactorCredential),
}

#[derive(Clone, Debug)]
pub(crate) enum PendingAuthRecord {
    LinkedAccount {
        source_user_id: String,
        provider: String,
        subject: String,
        created_at: Option<String>,
        updated_at: Option<String>,
    },
    SecureToken {
        source_user_id: String,
        token: SecretString,
        purpose: String,
        used_at: Option<String>,
        expires_at: String,
        created_at: String,
        updated_at: String,
    },
    FailedLoginAttempt(ImportedFailedLoginAttempt),
    TwoFactorCredential {
        source_user_id: String,
        secret: SecretString,
        confirmed_at: Option<String>,
        recovery_codes: Option<SecretString>,
        last_used_timestep: Option<i64>,
        created_at: String,
        updated_at: String,
    },
}

impl PendingAuthRecord {
    pub(crate) fn resolve(self, app_user_id: Option<i64>) -> crate::Result<DurableAuthRecord> {
        match self {
            Self::LinkedAccount {
                source_user_id,
                provider,
                subject,
                created_at,
                updated_at,
            } => Ok(DurableAuthRecord::LinkedAccount(ImportedLinkedAccount {
                app_user_id: required_owner(app_user_id, &source_user_id)?,
                provider,
                subject,
                created_at,
                updated_at,
            })),
            Self::SecureToken {
                source_user_id,
                token,
                purpose,
                used_at,
                expires_at,
                created_at,
                updated_at,
            } => Ok(DurableAuthRecord::SecureToken(ImportedSecureToken {
                app_user_id: required_owner(app_user_id, &source_user_id)?,
                token,
                purpose,
                used_at,
                expires_at,
                created_at,
                updated_at,
            })),
            Self::FailedLoginAttempt(attempt) => Ok(DurableAuthRecord::FailedLoginAttempt(attempt)),
            Self::TwoFactorCredential {
                source_user_id,
                secret,
                confirmed_at,
                recovery_codes,
                last_used_timestep,
                created_at,
                updated_at,
            } => Ok(DurableAuthRecord::TwoFactorCredential(
                ImportedTwoFactorCredential {
                    app_user_id: required_owner(app_user_id, &source_user_id)?,
                    secret,
                    confirmed_at,
                    recovery_codes,
                    last_used_timestep,
                    created_at,
                    updated_at,
                },
            )),
        }
    }

    pub(crate) fn source_user_id(&self) -> Option<&str> {
        match self {
            Self::LinkedAccount { source_user_id, .. }
            | Self::SecureToken { source_user_id, .. }
            | Self::TwoFactorCredential { source_user_id, .. } => Some(source_user_id),
            Self::FailedLoginAttempt(_) => None,
        }
    }
}

fn required_owner(owner: Option<i64>, source_user_id: &str) -> crate::Result<i64> {
    owner.ok_or_else(|| crate::Error::Conflict {
        resource: "durable authentication record owner".to_owned(),
        message: format!("source user {source_user_id} has no resolved application identity"),
    })
}
