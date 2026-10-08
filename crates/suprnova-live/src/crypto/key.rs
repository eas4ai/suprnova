//! Capability key material, validity windows, and derivation purposes.

use std::error::Error;
use std::fmt;

use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::identity::{KeyId, UnixMillis};

pub(crate) const DERIVED_KEY_BYTES: usize = 32;
const MINIMUM_ROOT_KEY_BYTES: usize = 32;
const MAXIMUM_ROOT_KEY_BYTES: usize = 64;
const HKDF_SALT_V1: &[u8] = b"suprnova-live/snapshot-hkdf/v1";
const SEED_INFO_V1: &[u8] = b"suprnova-live/seed-signature/v1";
const INSTANCE_INFO_V1: &[u8] = b"suprnova-live/instance-signature/v1";
const CHILD_PARAMETERS_INFO_V1: &[u8] = b"suprnova-live/child-params-signature/v1";
const CHILD_PARAMETERS_INFO_V2: &[u8] = b"suprnova-live/child-params-signature/v2";
const UPLOAD_GRANT_INFO_V1: &[u8] = b"suprnova-live/upload-grant/v1";
const ASYNC_SUBSCRIPTION_INFO_V1: &[u8] = b"suprnova-live/async-subscription/v1";
const RENDER_VARIANCE_INFO_V1: &[u8] = b"suprnova-live/render-variance/v1";
const RENDER_KEY_INFO_V1: &[u8] = b"suprnova-live/render-key/v1";
const RENDER_ENTRY_INFO_V1: &[u8] = b"suprnova-live/render-entry/v1";

/// Versioned purpose used to derive a capability MAC key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotPurpose {
    /// Public seed snapshot schema version 1.
    SeedV1,
    /// Instanced snapshot schema version 1.
    InstanceV1,
    /// Parent-to-child parameter capability schema version 1.
    ChildParametersV1,
    /// Exact-child-bound parent-to-child parameter capability schema version 2.
    ChildParametersV2,
    /// Temporary upload transfer-grant schema version 1.
    UploadGrantV1,
    /// Authorized asynchronous subscription descriptor schema version 1.
    AsyncSubscriptionV1,
    /// Render-cache variance material schema version 1.
    RenderVarianceV1,
    /// Render-cache lookup key schema version 1.
    RenderKeyV1,
    /// Render-cache stored entry integrity schema version 1.
    RenderEntryV1,
}

impl SnapshotPurpose {
    fn info(self) -> &'static [u8] {
        match self {
            Self::SeedV1 => SEED_INFO_V1,
            Self::InstanceV1 => INSTANCE_INFO_V1,
            Self::ChildParametersV1 => CHILD_PARAMETERS_INFO_V1,
            Self::ChildParametersV2 => CHILD_PARAMETERS_INFO_V2,
            Self::UploadGrantV1 => UPLOAD_GRANT_INFO_V1,
            Self::AsyncSubscriptionV1 => ASYNC_SUBSCRIPTION_INFO_V1,
            Self::RenderVarianceV1 => RENDER_VARIANCE_INFO_V1,
            Self::RenderKeyV1 => RENDER_KEY_INFO_V1,
            Self::RenderEntryV1 => RENDER_ENTRY_INFO_V1,
        }
    }
}

/// Closed reason for rejecting snapshot key configuration or verification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyErrorKind {
    /// Root key material contains fewer than 256 bits or exceeds its bound.
    WeakRootKey,
    /// Activation, signing, and verification deadlines are not ordered.
    InvalidKeyWindow,
    /// The configured ring contains more verification keys than permitted.
    TooManyKeys,
    /// Two configured records use the same key ID.
    DuplicateKeyId,
    /// No configured record matches the signed key ID.
    UnknownKey,
    /// The selected key's activation time has not arrived.
    KeyNotActive,
    /// The selected key's verification deadline has passed.
    KeyRetired,
    /// The canonical signature is not exactly 32 base64url-encoded bytes.
    InvalidSignatureEncoding,
    /// HMAC verification rejected the body, purpose, or key.
    SignatureMismatch,
    /// The fixed HKDF/HMAC construction could not initialize.
    DerivationFailure,
}

impl KeyErrorKind {
    /// Returns the stable safe machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WeakRootKey => "weak_root_key",
            Self::InvalidKeyWindow => "invalid_key_window",
            Self::TooManyKeys => "too_many_keys",
            Self::DuplicateKeyId => "duplicate_key_id",
            Self::UnknownKey => "unknown_key",
            Self::KeyNotActive => "key_not_active",
            Self::KeyRetired => "key_retired",
            Self::InvalidSignatureEncoding => "invalid_signature_encoding",
            Self::SignatureMismatch => "signature_mismatch",
            Self::DerivationFailure => "key_derivation_failure",
        }
    }
}

/// Redacted snapshot key configuration or verification error.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct KeyError {
    kind: KeyErrorKind,
}

impl KeyError {
    pub(crate) const fn new(kind: KeyErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed rejection reason.
    #[must_use]
    pub const fn kind(self) -> KeyErrorKind {
        self.kind
    }
}

impl fmt::Display for KeyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl fmt::Debug for KeyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for KeyError {}

/// Zeroizing root material used only as HKDF input.
#[derive(Clone)]
pub struct RootKey(Zeroizing<Vec<u8>>);

impl RootKey {
    /// Validates and takes ownership of bounded root key bytes.
    pub fn new(bytes: Vec<u8>) -> Result<Self, KeyError> {
        if bytes.len() < MINIMUM_ROOT_KEY_BYTES || bytes.len() > MAXIMUM_ROOT_KEY_BYTES {
            return Err(KeyError::new(KeyErrorKind::WeakRootKey));
        }
        Ok(Self(Zeroizing::new(bytes)))
    }

    pub(crate) fn derive(
        &self,
        purpose: SnapshotPurpose,
    ) -> Result<Zeroizing<[u8; DERIVED_KEY_BYTES]>, KeyError> {
        let hkdf = Hkdf::<Sha256>::new(Some(HKDF_SALT_V1), self.0.as_slice());
        let mut output = Zeroizing::new([0_u8; DERIVED_KEY_BYTES]);
        hkdf.expand(purpose.info(), output.as_mut())
            .map_err(|_| KeyError::new(KeyErrorKind::DerivationFailure))?;
        Ok(output)
    }
}

impl fmt::Debug for RootKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<RootKey:redacted>")
    }
}

/// One root key with separate signing and verification deadlines.
#[derive(Clone)]
pub struct KeyRecord {
    key_id: KeyId,
    root_key: RootKey,
    active_from: UnixMillis,
    sign_until: UnixMillis,
    verify_until: UnixMillis,
}

impl KeyRecord {
    /// Creates a key whose intervals are `[active_from, sign_until)` and
    /// `[active_from, verify_until)`.
    pub fn new(
        key_id: KeyId,
        root_key: RootKey,
        active_from: UnixMillis,
        sign_until: UnixMillis,
        verify_until: UnixMillis,
    ) -> Result<Self, KeyError> {
        if active_from >= sign_until || sign_until > verify_until {
            return Err(KeyError::new(KeyErrorKind::InvalidKeyWindow));
        }
        Ok(Self {
            key_id,
            root_key,
            active_from,
            sign_until,
            verify_until,
        })
    }

    /// Returns the configured public key identifier.
    #[must_use]
    pub const fn key_id(&self) -> &KeyId {
        &self.key_id
    }

    pub(crate) fn ensure_can_sign(&self, now: UnixMillis) -> Result<(), KeyError> {
        if now < self.active_from {
            return Err(KeyError::new(KeyErrorKind::KeyNotActive));
        }
        if now >= self.sign_until {
            return Err(KeyError::new(KeyErrorKind::KeyRetired));
        }
        Ok(())
    }

    pub(crate) fn ensure_can_verify(&self, now: UnixMillis) -> Result<(), KeyError> {
        if now < self.active_from {
            return Err(KeyError::new(KeyErrorKind::KeyNotActive));
        }
        if now >= self.verify_until {
            return Err(KeyError::new(KeyErrorKind::KeyRetired));
        }
        Ok(())
    }

    pub(crate) fn derive(
        &self,
        purpose: SnapshotPurpose,
    ) -> Result<Zeroizing<[u8; DERIVED_KEY_BYTES]>, KeyError> {
        self.root_key.derive(purpose)
    }
}

impl fmt::Debug for KeyRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<KeyRecord:redacted>")
    }
}
