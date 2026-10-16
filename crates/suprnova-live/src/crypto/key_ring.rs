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
        self.mac_with(purpose, |writer| {
            for part in parts {
                writer.part(&[part]);
            }
        })
    }

    /// The same MAC with the parts streamed in rather than materialized:
    /// `write` receives a [`PartWriter`] and calls `part` once per logical
    /// part. A part given as several pieces produces exactly the bytes the
    /// concatenated part would through [`Self::mac`], so a caller can avoid
    /// building tag-prefixed buffers without changing any digest. Nothing
    /// here allocates: the derived key is a stack array and the HMAC state
    /// lives on the stack.
    pub(crate) fn mac_with(
        &self,
        purpose: SnapshotPurpose,
        write: impl FnOnce(&mut PartWriter<'_>),
    ) -> Result<[u8; 32], KeyError> {
        let derived = self.active.derive(purpose)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(derived.as_ref())
            .map_err(|_| KeyError::new(KeyErrorKind::DerivationFailure))?;
        write(&mut PartWriter { mac: &mut mac });
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

/// Feeds length-prefixed parts into a purpose-separated MAC. See
/// [`SnapshotKeyRing::mac_with`].
pub(crate) struct PartWriter<'m> {
    mac: &'m mut Hmac<Sha256>,
}

impl PartWriter<'_> {
    /// One part given as pieces: the 8-byte big-endian total length, then
    /// each piece in order.
    pub(crate) fn part(&mut self, pieces: &[&[u8]]) {
        let total: u64 = pieces.iter().map(|piece| piece.len() as u64).sum();
        self.mac.update(&total.to_be_bytes());
        for piece in pieces {
            self.mac.update(piece);
        }
    }

    /// One part whose `len` bytes arrive through `write`'s sink calls. The
    /// caller is responsible for `len` matching what it writes; a mismatch
    /// is a programming error and a debug assertion catches it.
    ///
    /// The debug assertion is a backstop, not the guarantee. In release it
    /// is gone, and a `len` that overstated or understated what `write`
    /// produced would prefix the MAC with a length that does not describe
    /// its part - which is the one thing the length framing exists to make
    /// impossible. What actually holds the pair together is a test per
    /// caller. The only caller today is the render key's variance
    /// descriptor, and
    /// `the_streamed_canonical_form_matches_its_declared_length_and_the_expected_bytes`
    /// (`tests/render_cache_variance.rs`) pins `canonical_len` and
    /// `write_canonical` against bytes written out by hand for every
    /// dimension shape, rather than against each other. A new dimension
    /// variant, or a second caller, needs its own such test.
    pub(crate) fn part_streamed(&mut self, len: usize, write: impl FnOnce(&mut dyn FnMut(&[u8]))) {
        self.mac.update(&(len as u64).to_be_bytes());
        let mac = &mut *self.mac;
        let mut written = 0_usize;
        let mut sink = |bytes: &[u8]| {
            mac.update(bytes);
            written += bytes.len();
        };
        write(&mut sink);
        debug_assert_eq!(
            written, len,
            "streamed part length must match its declared length"
        );
    }
}

impl fmt::Debug for SnapshotKeyRing {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<SnapshotKeyRing:redacted>")
    }
}
