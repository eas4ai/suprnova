//! Ed25519 keys, fingerprints and signatures over a component's verification
//! hash (REG-024), key handover (REG-033) and the author's signing (REG-020).

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sha2::{Digest as _, Sha256};

use super::statement::Digest;
use super::{RegistryError, Result};

/// A library's public key, written `ed25519:` and the standard padded
/// base64 of its 32 bytes.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PublicKey([u8; 32]);

impl PublicKey {
    /// Reads `ed25519:<base64>`.
    pub fn parse(text: &str) -> Result<Self> {
        let encoded = text.strip_prefix("ed25519:").ok_or_else(|| {
            RegistryError::Invalid(format!("public key `{text}` is not `ed25519:` and base64"))
        })?;
        let bytes = STANDARD.decode(encoded).map_err(|error| {
            RegistryError::Invalid(format!("public key is not standard base64: {error}"))
        })?;
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| RegistryError::Invalid("public key is not 32 bytes".to_owned()))?;
        Ok(PublicKey(bytes))
    }

    /// The key as written in `library.json`.
    pub fn encode(&self) -> String {
        format!("ed25519:{}", STANDARD.encode(self.0))
    }

    /// `sha256:` and the lowercase hex digest of the key bytes.
    pub fn fingerprint(&self) -> Fingerprint {
        let hex: String = Sha256::digest(self.0)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Fingerprint(format!("sha256:{hex}"))
    }

    /// The raw key bytes.
    pub fn bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({})", self.fingerprint())
    }
}

/// A key's fingerprint, what the plan shows and the project pins by.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fingerprint(String);

impl Fingerprint {
    /// Reads a fingerprint written as `sha256:` and 64 lowercase hex characters.
    pub fn parse(text: &str) -> Option<Self> {
        Digest::parse(text).map(|digest| Fingerprint(digest.as_str().to_owned()))
    }

    /// The fingerprint as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An Ed25519 signature, written as the standard padded base64 of its 64
/// bytes and nothing else.
#[derive(Clone, PartialEq, Eq)]
pub struct Signature([u8; 64]);

impl Signature {
    /// Reads the base64 `manifest.sig` holds, surrounding whitespace
    /// excluded.
    pub fn parse(text: &str) -> Result<Self> {
        let bytes = STANDARD.decode(text.trim()).map_err(|error| {
            RegistryError::Invalid(format!("signature is not standard base64: {error}"))
        })?;
        let bytes: [u8; 64] = bytes
            .try_into()
            .map_err(|_| RegistryError::Invalid("signature is not 64 bytes".to_owned()))?;
        Ok(Signature(bytes))
    }

    /// The signature as `manifest.sig` writes it.
    pub fn encode(&self) -> String {
        STANDARD.encode(self.0)
    }

    /// The raw signature bytes.
    pub fn bytes(&self) -> &[u8; 64] {
        &self.0
    }
}

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Signature(..)")
    }
}

/// A former key vouching for the key that replaced it (REG-033): the
/// statement signed is the new key's fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyHandover {
    /// The former key.
    pub from: PublicKey,
    /// The fingerprint of the key it hands over to.
    pub to: Fingerprint,
    /// The former key's signature over the ASCII bytes of `to`.
    pub signature: Signature,
}

/// A library's private key, held by its author; never written inside the
/// project (REG-018, REG-020).
pub struct SecretKey([u8; 32]);

impl SecretKey {
    /// The raw secret bytes.
    pub fn bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Wraps 32 secret bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        SecretKey(bytes)
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretKey(..)")
    }
}

/// Verifies `signature` by `key` over the ASCII bytes of `hash`.
pub fn verify(key: &PublicKey, hash: &Digest, signature: &Signature) -> Result<()> {
    let _ = (key, hash, signature);
    Err(RegistryError::NotBuilt("signature verification"))
}

/// Verifies a key handover: `handover.from` signed `handover.to`, and
/// `handover.to` is `new_key`'s fingerprint.
pub fn verify_handover(handover: &KeyHandover, new_key: &PublicKey) -> Result<()> {
    let _ = (handover, new_key);
    Err(RegistryError::NotBuilt("key handover verification"))
}

/// Signs the ASCII bytes of `hash` with `key`.
pub fn sign(key: &SecretKey, hash: &Digest) -> Result<Signature> {
    let _ = (key, hash);
    Err(RegistryError::NotBuilt("signing"))
}

/// Makes a new key pair.
pub fn generate() -> Result<(SecretKey, PublicKey)> {
    Err(RegistryError::NotBuilt("key generation"))
}

#[cfg(test)]
mod tests {
    use super::{PublicKey, Signature};

    #[test]
    fn keys_and_signatures_round_trip_through_their_written_forms() {
        let key = PublicKey([7u8; 32]);
        let written = key.encode();
        assert!(written.starts_with("ed25519:"));
        assert_eq!(PublicKey::parse(&written).expect("parses"), key);
        assert!(key.fingerprint().as_str().starts_with("sha256:"));
        assert!(PublicKey::parse("ed25519:AAAA").is_err());
        assert!(PublicKey::parse("rsa:AAAA").is_err());
        let signature = Signature([9u8; 64]);
        assert_eq!(
            Signature::parse(&format!("{}\n", signature.encode())).expect("parses"),
            signature
        );
        assert!(Signature::parse("AAAA").is_err());
    }
}
