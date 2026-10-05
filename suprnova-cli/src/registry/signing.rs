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
            RegistryError::Invalid(format!(
                "public key `{}` is not `ed25519:` and base64",
                super::printable(text)
            ))
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

    /// Wraps 32 key bytes. Whether they are a valid Ed25519 point is
    /// decided when a signature is verified against them.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        PublicKey(bytes)
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
    /// Reads the base64 `manifest.sig` holds, and nothing else: REG-024
    /// gives the file no trailing newline and no other byte.
    pub fn parse(text: &str) -> Result<Self> {
        if text.trim() != text {
            return Err(RegistryError::Invalid(
                "a signature is base64 with nothing around it".to_owned(),
            ));
        }
        let bytes = STANDARD.decode(text).map_err(|error| {
            RegistryError::Invalid(format!("signature is not standard base64: {error}"))
        })?;
        let bytes: [u8; 64] = bytes
            .try_into()
            .map_err(|_| RegistryError::Invalid("signature is not 64 bytes".to_owned()))?;
        Ok(Signature(bytes))
    }

    /// The same as [`Signature::parse`]: every signature the registry reads,
    /// in `manifest.sig` or a JSON value (REG-033), has nothing around it.
    pub fn parse_strict(text: &str) -> Result<Self> {
        Signature::parse(text)
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

/// A former key vouching for the key that replaced it (REG-033). The
/// statement signed is [`handover_statement`]: the library's `source` and
/// the new key's fingerprint, so a handover moves one library's key and no
/// other library's that the same former key signed for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyHandover {
    /// The former key.
    pub from: PublicKey,
    /// The fingerprint of the key it hands over to.
    pub to: Fingerprint,
    /// The former key's signature over the handover statement naming the
    /// library and `to`.
    pub signature: Signature,
}

/// The format a handover statement names.
pub const HANDOVER_FORMAT: &str = "suprnova-key-handover/1";

/// What a former key signs to hand a library over to a new key: one JSON
/// object with no whitespace, `{"format":"suprnova-key-handover/1",
/// "library":"<source>","next":"<fingerprint>"}`, where `library` is the
/// `source` of the `library.json` that carries the handover.
pub fn handover_statement(library: &str, next: &Fingerprint) -> String {
    let quoted = |value: &str| serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned());
    format!(
        "{{\"format\":{},\"library\":{},\"next\":{}}}",
        quoted(HANDOVER_FORMAT),
        quoted(library),
        quoted(next.as_str())
    )
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

    /// The public key of this secret key, the one `library.json` names.
    pub fn public_key(&self) -> PublicKey {
        PublicKey(
            ed25519_dalek::SigningKey::from_bytes(&self.0)
                .verifying_key()
                .to_bytes(),
        )
    }
}

impl Drop for SecretKey {
    /// Overwrites the secret when the key is dropped, so it does not linger
    /// in freed memory.
    fn drop(&mut self) {
        self.0 = [0; 32];
        std::hint::black_box(&self.0);
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretKey(..)")
    }
}

/// Verifies `signature` by `key` over the ASCII bytes of `hash`.
///
/// Strict verification: a key of small order or a signature in a
/// non-canonical encoding is refused, so one component has exactly one
/// valid signature per key.
pub fn verify(key: &PublicKey, hash: &Digest, signature: &Signature) -> Result<()> {
    verify_message(key, hash.as_str().as_bytes(), signature).map_err(|reason| {
        RegistryError::Invalid(format!(
            "the signature does not verify over {hash} with the key {}: {reason}",
            key.fingerprint()
        ))
    })
}

/// Verifies a key handover: `handover.from` signed the handover statement
/// for `library` and `handover.to`, and `handover.to` is `new_key`'s
/// fingerprint.
pub fn verify_handover(handover: &KeyHandover, new_key: &PublicKey, library: &str) -> Result<()> {
    if handover.to != new_key.fingerprint() {
        return Err(RegistryError::Invalid(format!(
            "the handover from {} names {}, not the new key {}",
            handover.from.fingerprint(),
            handover.to,
            new_key.fingerprint()
        )));
    }
    verify_message(
        &handover.from,
        handover_statement(library, &handover.to).as_bytes(),
        &handover.signature,
    )
    .map_err(|reason| {
        RegistryError::Invalid(format!(
            "the handover from {} to {} for {} does not verify: {reason}",
            handover.from.fingerprint(),
            handover.to,
            super::printable(library)
        ))
    })
}

fn verify_message(
    key: &PublicKey,
    message: &[u8],
    signature: &Signature,
) -> std::result::Result<(), String> {
    let verifying = ed25519_dalek::VerifyingKey::from_bytes(key.bytes())
        .map_err(|_| "the key is not a valid Ed25519 public key".to_owned())?;
    let signature = ed25519_dalek::Signature::from_bytes(signature.bytes());
    verifying
        .verify_strict(message, &signature)
        .map_err(|_| "the signature is not the key's".to_owned())
}

/// Signs the ASCII bytes of `hash` with `key`. Ed25519 signing is
/// deterministic, so one tree and one key always sign to the same bytes
/// (REG-020).
pub fn sign(key: &SecretKey, hash: &Digest) -> Result<Signature> {
    Ok(sign_message(key, hash.as_str().as_bytes()))
}

/// Signs a handover from `former` to `new_key` (REG-033): `former` signs
/// the ASCII bytes of the new key's fingerprint.
pub fn sign_handover(
    former: &SecretKey,
    new_key: &PublicKey,
    library: &str,
) -> Result<KeyHandover> {
    let to = new_key.fingerprint();
    let signature = sign_message(former, handover_statement(library, &to).as_bytes());
    Ok(KeyHandover {
        from: former.public_key(),
        to,
        signature,
    })
}

fn sign_message(key: &SecretKey, message: &[u8]) -> Signature {
    use ed25519_dalek::Signer as _;
    let signing = ed25519_dalek::SigningKey::from_bytes(key.bytes());
    Signature(signing.sign(message).to_bytes())
}

/// Makes a new key pair from the operating system's random source.
pub fn generate() -> Result<(SecretKey, PublicKey)> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| {
        RegistryError::Io(format!(
            "the operating system's random source is unavailable: {error}"
        ))
    })?;
    let secret = SecretKey::from_bytes(bytes);
    bytes = [0; 32];
    std::hint::black_box(&bytes);
    let public = secret.public_key();
    Ok((secret, public))
}

#[cfg(test)]
mod tests {
    use super::{
        PublicKey, SecretKey, Signature, generate, sign, sign_handover, verify, verify_handover,
    };
    use crate::registry::statement::Digest;

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
            Signature::parse(&signature.encode()).expect("parses"),
            signature
        );
        assert!(
            Signature::parse(&format!("{}\n", signature.encode())).is_err(),
            "a trailing newline is not part of a signature"
        );
        assert!(Signature::parse("AAAA").is_err());
    }

    #[test]
    fn a_signature_verifies_only_over_its_hash_with_its_key() {
        let secret = SecretKey::from_bytes([1u8; 32]);
        let public = secret.public_key();
        let hash = Digest::of(b"statement");
        let signature = sign(&secret, &hash).expect("sign");
        verify(&public, &hash, &signature).expect("verifies");
        assert_eq!(sign(&secret, &hash).expect("sign again"), signature);
        assert!(verify(&public, &Digest::of(b"other"), &signature).is_err());
        let other = SecretKey::from_bytes([2u8; 32]).public_key();
        assert!(verify(&other, &hash, &signature).is_err());
        let (fresh, fresh_public) = generate().expect("generate");
        assert_eq!(fresh.public_key(), fresh_public);
    }

    #[test]
    fn a_handover_verifies_only_for_the_key_it_names() {
        let former = SecretKey::from_bytes([3u8; 32]);
        let new_key = SecretKey::from_bytes([4u8; 32]).public_key();
        let handover =
            sign_handover(&former, &new_key, "github.com/acme/acme-ui").expect("handover");
        verify_handover(&handover, &new_key, "github.com/acme/acme-ui").expect("verifies");
        let stranger = SecretKey::from_bytes([5u8; 32]).public_key();
        assert!(verify_handover(&handover, &stranger, "github.com/acme/acme-ui").is_err());
        let mut forged = handover.clone();
        forged.from = stranger;
        assert!(verify_handover(&forged, &new_key, "github.com/acme/acme-ui").is_err());
        assert!(
            verify_handover(&handover, &new_key, "github.com/acme/other").is_err(),
            "a handover moved another library's key"
        );
        assert_eq!(
            super::handover_statement("github.com/acme/acme-ui", &new_key.fingerprint()),
            format!(
                "{{\"format\":\"suprnova-key-handover/1\",\"library\":\"github.com/acme/acme-ui\",\"next\":\"{}\"}}",
                new_key.fingerprint()
            )
        );
        assert!(Signature::parse_strict(&format!(" {}", handover.signature.encode())).is_err());
    }
}
