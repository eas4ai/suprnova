//! Fixed snapshot signatures and signed key selection metadata.

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Serialize;

use super::{KeyError, KeyErrorKind};
use crate::identity::KeyId;

const SIGNATURE_BYTES: usize = 32;
const SIGNATURE_BASE64URL_BYTES: usize = 43;

/// A 32-byte HMAC-SHA-256 tag encoded as unpadded base64url on the wire.
#[derive(Clone, Eq, PartialEq)]
pub struct SnapshotSignature([u8; SIGNATURE_BYTES]);

impl SnapshotSignature {
    /// Parses a canonical fixed-length unpadded base64url signature.
    pub fn parse(encoded: &str) -> Result<Self, KeyError> {
        if encoded.len() != SIGNATURE_BASE64URL_BYTES || encoded.contains('=') {
            return Err(KeyError::new(KeyErrorKind::InvalidSignatureEncoding));
        }
        let decoded = URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| KeyError::new(KeyErrorKind::InvalidSignatureEncoding))?;
        let bytes: [u8; SIGNATURE_BYTES] = decoded
            .try_into()
            .map_err(|_| KeyError::new(KeyErrorKind::InvalidSignatureEncoding))?;
        if URL_SAFE_NO_PAD.encode(bytes) != encoded {
            return Err(KeyError::new(KeyErrorKind::InvalidSignatureEncoding));
        }
        Ok(Self(bytes))
    }

    pub(crate) const fn from_bytes(bytes: [u8; SIGNATURE_BYTES]) -> Self {
        Self(bytes)
    }

    /// Returns the fixed signature bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; SIGNATURE_BYTES] {
        &self.0
    }

    /// Encodes the signature as canonical unpadded base64url.
    #[must_use]
    pub fn to_base64url(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.0)
    }
}

impl Serialize for SnapshotSignature {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_base64url())
    }
}

impl fmt::Debug for SnapshotSignature {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<SnapshotSignature:redacted>")
    }
}

/// Signature plus the explicit key ID required for bounded rotation lookup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignedMac {
    key_id: KeyId,
    signature: SnapshotSignature,
}

impl SignedMac {
    pub(crate) const fn new(key_id: KeyId, signature: SnapshotSignature) -> Self {
        Self { key_id, signature }
    }

    /// Returns the signing key ID that is also bound inside snapshot bodies.
    #[must_use]
    pub const fn key_id(&self) -> &KeyId {
        &self.key_id
    }

    /// Returns the fixed HMAC proof.
    #[must_use]
    pub const fn signature(&self) -> &SnapshotSignature {
        &self.signature
    }
}
