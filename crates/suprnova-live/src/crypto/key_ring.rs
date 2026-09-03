//! Bounded signing and overlapping verification key ring.

use std::fmt;

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use super::{KeyError, KeyErrorKind, KeyRecord, SignedMac, SnapshotPurpose, SnapshotSignature};
use crate::identity::{KeyId, UnixMillis};

const MAXIMUM_KEY_RECORDS: usize = 8;

/// One active signing key plus bounded overlapping verification keys.
pub struct SnapshotKeyRing {
    active: KeyRecord,
    verification: Vec<KeyRecord>,
}

impl SnapshotKeyRing {
    /// Builds a bounded ring and rejects duplicate key IDs.
    pub fn new(active: KeyRecord, verification: Vec<KeyRecord>) -> Result<Self, KeyError> {
        if verification.len().saturating_add(1) > MAXIMUM_KEY_RECORDS {
            return Err(KeyError::new(KeyErrorKind::TooManyKeys));
        }
        if verification
            .iter()
            .any(|record| record.key_id() == active.key_id())
        {
            return Err(KeyError::new(KeyErrorKind::DuplicateKeyId));
        }
        for (index, record) in verification.iter().enumerate() {
            if verification[index + 1..]
                .iter()
                .any(|candidate| candidate.key_id() == record.key_id())
            {
                return Err(KeyError::new(KeyErrorKind::DuplicateKeyId));
            }
        }
        Ok(Self {
            active,
            verification,
        })
    }

    /// Returns the public identifier of the key used for new signatures.
    #[must_use]
    pub const fn active_key_id(&self) -> &KeyId {
        self.active.key_id()
    }

    /// Signs bytes with the active key and a purpose/version-derived MAC key.
    pub fn sign(
        &self,
        purpose: SnapshotPurpose,
        canonical_body: &[u8],
        now: UnixMillis,
    ) -> Result<SignedMac, KeyError> {
        self.active.ensure_can_sign(now)?;
        let derived = self.active.derive(purpose)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(derived.as_ref())
            .map_err(|_| KeyError::new(KeyErrorKind::DerivationFailure))?;
        mac.update(canonical_body);
        let tag = mac.finalize().into_bytes();
        let mut signature = [0_u8; 32];
        signature.copy_from_slice(&tag);
        Ok(SignedMac::new(
            self.active.key_id().clone(),
            SnapshotSignature::from_bytes(signature),
        ))
    }

    /// Verifies a fixed HMAC tag with bounded key lookup and RustCrypto's
    /// constant-time verifier.
    pub fn verify(
        &self,
        key_id: &KeyId,
        purpose: SnapshotPurpose,
        canonical_body: &[u8],
        signature: &SnapshotSignature,
        now: UnixMillis,
    ) -> Result<(), KeyError> {
        let record = self
            .find(key_id)
            .ok_or_else(|| KeyError::new(KeyErrorKind::UnknownKey))?;
        record.ensure_can_verify(now)?;
        let derived = record.derive(purpose)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(derived.as_ref())
            .map_err(|_| KeyError::new(KeyErrorKind::DerivationFailure))?;
        mac.update(canonical_body);
        mac.verify_slice(signature.as_bytes())
            .map_err(|_| KeyError::new(KeyErrorKind::SignatureMismatch))
    }

    /// Purpose-separated MAC over length-prefixed parts, for digests that
    /// carry no validity window (render-cache keys, variance material, and
    /// stored-entry integrity). Signatures keep using `sign` and `verify`.
    pub(crate) fn mac(
        &self,
        purpose: SnapshotPurpose,
        parts: &[&[u8]],
    ) -> Result<[u8; 32], KeyError> {
        let derived = self.active.derive(purpose)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(derived.as_ref())
            .map_err(|_| KeyError::new(KeyErrorKind::DerivationFailure))?;
        for part in parts {
            mac.update(&(part.len() as u64).to_be_bytes());
            mac.update(part);
        }
        let tag = mac.finalize().into_bytes();
        let mut out = [0_u8; 32];
        out.copy_from_slice(&tag);
        Ok(out)
    }

    fn find(&self, key_id: &KeyId) -> Option<&KeyRecord> {
        if self.active.key_id() == key_id {
            return Some(&self.active);
        }
        self.verification
            .iter()
            .find(|record| record.key_id() == key_id)
    }
}

impl fmt::Debug for SnapshotKeyRing {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<SnapshotKeyRing:redacted>")
    }
}
