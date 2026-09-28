//! Secret-bearing session token.

use std::{fmt, hash::Hash};

use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

const BASE58_ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const RANDOM_TOKEN_LENGTH: usize = 32;

/// An opaque session credential that redacts itself in logs.
///
/// The plaintext is held only in [`SecretString`]. Call
/// [`SessionToken::expose_secret`] at the explicit boundary where a credential
/// must be sent to a client or hashed for a lookup.
#[derive(Clone)]
pub struct SessionToken(SecretString);

impl SessionToken {
    /// Wraps an existing plaintext session credential.
    #[must_use]
    pub fn new(token: &str) -> Self {
        Self(SecretString::from(token.to_owned()))
    }

    /// Creates a random opaque session credential with at least 128 bits of entropy.
    #[must_use]
    pub fn new_random() -> Self {
        let mut token = String::with_capacity(RANDOM_TOKEN_LENGTH);
        append_random_base58(&mut token);
        Self(SecretString::from(token))
    }

    /// Explicitly exposes the credential for storage, transmission, or comparison.
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        self.0.expose_secret()
    }

    /// Consumes this token and returns the secret container.
    #[must_use]
    pub fn into_secret(self) -> SecretString {
        self.0
    }

    /// Returns the lowercase hexadecimal SHA-256 digest used for session lookup.
    #[must_use]
    pub fn token_hash(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.expose_secret().as_bytes());
        hex_encode(&hasher.finalize())
    }

    /// Returns whether this token matches a stored SHA-256 token digest.
    #[must_use]
    pub fn verify_hash(&self, stored_hash: &str) -> bool {
        self.token_hash()
            .as_bytes()
            .ct_eq(stored_hash.as_bytes())
            .into()
    }
}

impl Default for SessionToken {
    fn default() -> Self {
        Self::new_random()
    }
}

impl From<String> for SessionToken {
    fn from(value: String) -> Self {
        Self(SecretString::from(value))
    }
}

impl From<&str> for SessionToken {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<SecretString> for SessionToken {
    fn from(value: SecretString) -> Self {
        Self(value)
    }
}

impl From<SessionToken> for SecretString {
    fn from(value: SessionToken) -> Self {
        value.into_secret()
    }
}

impl fmt::Display for SessionToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

impl fmt::Debug for SessionToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SessionToken([REDACTED])")
    }
}

impl PartialEq for SessionToken {
    fn eq(&self, other: &Self) -> bool {
        self.expose_secret() == other.expose_secret()
    }
}

impl Eq for SessionToken {}

impl Hash for SessionToken {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.expose_secret().hash(state);
    }
}

impl serde::Serialize for SessionToken {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.expose_secret())
    }
}

impl<'de> serde::Deserialize<'de> for SessionToken {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let token = String::deserialize(deserializer)?;
        Ok(Self::from(token))
    }
}

fn append_random_base58(token: &mut String) {
    let mut random = [0_u8; 48];
    let mut index = random.len();

    while token.len() < RANDOM_TOKEN_LENGTH {
        if index == random.len() {
            for byte in &mut random {
                *byte = rand::random();
            }
            index = 0;
        }

        let byte = random[index];
        index += 1;
        if byte < 232 {
            token.push(char::from(BASE58_ALPHABET[usize::from(byte % 58)]));
        }
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut encoded = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}
