//! Purpose-bound authenticated encryption.
//!
//! [`AeadEncryptor`](crate::crypto::AeadEncryptor) uses AES-256-GCM. The key is exactly 32 bytes and must be
//! supplied by the application from a secret-management system. Ciphertexts
//! use the authenticated format `[version: u8 | nonce: 12 bytes |
//! ciphertext || tag: at least 16 bytes]`; the nonce is freshly generated for
//! every encryption operation.

use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use rand::{RngCore, rngs::OsRng};

use crate::{Error, Result};

const FORMAT_VERSION: u8 = 1;
const NONCE_LENGTH: usize = 12;
const TAG_LENGTH: usize = 16;

/// The domain in which an encrypted value is used.
///
/// The associated-data labels are stable compatibility values. Changing one
/// is a deliberate key-rotation and data-migration decision, not a cosmetic
/// rename.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CryptoPurpose {
    /// Short-lived state used while completing an authentication ceremony.
    CeremonyState,
    /// A user's encrypted two-factor authenticator secret. Its label remains
    /// `suprnova:2fa:secret:v1` for unchanged ciphertext migration.
    TwoFactorSecret,
    /// Encrypted one-time two-factor recovery codes. Its label remains
    /// `suprnova:2fa:recovery:v1` for unchanged ciphertext migration.
    TwoFactorRecovery,
    /// An encrypted provider access or refresh token.
    ProviderToken,
    /// An encrypted application refresh token.
    RefreshToken,
    /// An encrypted session-grant carrier.
    SessionGrant,
}

impl CryptoPurpose {
    /// Return the stable associated-data label for this purpose.
    #[must_use]
    pub const fn label(self) -> &'static [u8] {
        self.label_str().as_bytes()
    }

    /// Return the stable associated-data label as UTF-8 text.
    #[must_use]
    pub const fn label_str(self) -> &'static str {
        match self {
            Self::CeremonyState => "magnetar/crypto/ceremony-state/v1",
            Self::TwoFactorSecret => "suprnova:2fa:secret:v1",
            Self::TwoFactorRecovery => "suprnova:2fa:recovery:v1",
            Self::ProviderToken => "magnetar/crypto/provider-token/v1",
            Self::RefreshToken => "magnetar/crypto/refresh-token/v1",
            Self::SessionGrant => "magnetar/crypto/session-grant/v1",
        }
    }
}

/// An authenticated encryptor backed by AES-256-GCM.
///
/// Construct this with a secret, uniformly random 32-byte key. The key is
/// retained only by this encryptor and is never serialized or included in an
/// error. A ciphertext's purpose is authenticated as associated data, so a
/// ciphertext cannot be decrypted under another [`CryptoPurpose`].
pub struct AeadEncryptor {
    cipher: Aes256Gcm,
}

impl AeadEncryptor {
    /// Construct an encryptor from a 32-byte AES-256 key.
    #[must_use]
    pub fn new(key: [u8; 32]) -> Self {
        // Aes256Gcm accepts every 32-byte key, so this cannot fail.
        let cipher =
            Aes256Gcm::new_from_slice(&key).expect("AES-256-GCM accepts exactly 32-byte keys");
        Self { cipher }
    }
}

impl Encryptor for AeadEncryptor {
    fn encrypt(&self, purpose: CryptoPurpose, plaintext: &[u8]) -> Result<Vec<u8>> {
        let mut nonce = [0_u8; NONCE_LENGTH];
        OsRng
            .try_fill_bytes(&mut nonce)
            .map_err(|_| Error::Internal {
                message: "secure random source unavailable".to_owned(),
            })?;

        let encrypted = self
            .cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: purpose.label(),
                },
            )
            .map_err(|_| Error::Internal {
                message: "authenticated encryption failed".to_owned(),
            })?;

        let mut ciphertext = Vec::with_capacity(1 + NONCE_LENGTH + encrypted.len());
        ciphertext.push(FORMAT_VERSION);
        ciphertext.extend_from_slice(&nonce);
        ciphertext.extend_from_slice(&encrypted);
        Ok(ciphertext)
    }

    fn decrypt(&self, purpose: CryptoPurpose, ciphertext: &[u8]) -> Result<Vec<u8>> {
        if ciphertext.len() < 1 + NONCE_LENGTH + TAG_LENGTH || ciphertext[0] != FORMAT_VERSION {
            return Err(malformed_ciphertext());
        }

        let nonce = Nonce::from_slice(&ciphertext[1..1 + NONCE_LENGTH]);
        self.cipher
            .decrypt(
                nonce,
                Payload {
                    msg: &ciphertext[1 + NONCE_LENGTH..],
                    aad: purpose.label(),
                },
            )
            .map_err(|_| malformed_ciphertext())
    }
}

/// The encryption boundary consumed by Magnetar features.
pub trait Encryptor: Send + Sync {
    /// Encrypt plaintext under an explicit purpose.
    fn encrypt(&self, purpose: CryptoPurpose, plaintext: &[u8]) -> Result<Vec<u8>>;

    /// Decrypt ciphertext under an explicit purpose.
    fn decrypt(&self, purpose: CryptoPurpose, ciphertext: &[u8]) -> Result<Vec<u8>>;
}

fn malformed_ciphertext() -> Error {
    Error::InvalidInput {
        field: "ciphertext".to_owned(),
        message: "malformed or unauthenticated ciphertext".to_owned(),
    }
}
