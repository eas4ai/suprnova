//! Bounded mount request, document identity, flags, and publishable output types.

use std::collections::{BTreeMap, HashSet};
use std::fmt;

use super::{MountError, MountErrorKind};
use crate::canonical::CanonicalValue;
use crate::identity::{InstanceId, Revision, UnixMillis};
use crate::view::{MountMetadata, TrustedHtml};

const MAX_KEY_BYTES: usize = 128;
const MAX_FLAG_NAME_BYTES: usize = 32;
const MAX_FLAG_VALUE_BYTES: usize = 1_024;
const DEFAULT_MAX_DOCUMENT_MOUNTS: usize = 128;
const HARD_MAX_DOCUMENT_MOUNTS: usize = 1_024;
pub(crate) const HARD_MAX_FLAGS: usize = 64;

/// Stable non-secret identity for one mount within a rendered document.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DocumentMountKey(String);

impl DocumentMountKey {
    /// Parses a bounded unreserved document-local key.
    pub fn parse(value: &str) -> Result<Self, MountError> {
        let valid = !value.is_empty()
            && value.len() <= MAX_KEY_BYTES
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
            });
        if !valid {
            return Err(MountError::new(MountErrorKind::MetadataTooLarge));
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the validated key text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Document-local key collector preventing ambiguous duplicate island identities.
#[derive(Debug)]
pub struct DocumentMountScope {
    keys: HashSet<DocumentMountKey>,
    max_keys: usize,
}

impl DocumentMountScope {
    /// Creates an empty document-local scope with conservative standard capacity.
    #[must_use]
    pub fn new() -> Self {
        Self {
            keys: HashSet::new(),
            max_keys: DEFAULT_MAX_DOCUMENT_MOUNTS,
        }
    }

    /// Creates an empty scope with a non-zero capacity below the engine ceiling.
    pub fn with_limit(max_keys: usize) -> Result<Self, MountError> {
        if !(1..=HARD_MAX_DOCUMENT_MOUNTS).contains(&max_keys) {
            return Err(MountError::new(MountErrorKind::InvalidConfiguration));
        }
        Ok(Self {
            keys: HashSet::new(),
            max_keys,
        })
    }

    pub(crate) fn reserve(&mut self, key: DocumentMountKey) -> Result<(), MountError> {
        if self.keys.contains(&key) {
            return Err(MountError::new(MountErrorKind::DuplicateDocumentKey));
        }
        if self.keys.len() >= self.max_keys {
            return Err(MountError::new(MountErrorKind::DocumentCapacity));
        }
        self.keys.insert(key);
        Ok(())
    }

    pub(crate) fn release(&mut self, key: &DocumentMountKey) {
        self.keys.remove(key);
    }
}

impl Default for DocumentMountScope {
    fn default() -> Self {
        Self::new()
    }
}

/// Bounded inert strings emitted only as `data-suprnova-live-flag-*` attributes.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MountFlags(BTreeMap<String, String>);

impl MountFlags {
    /// Creates no inert mount flags.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Validates inert names and printable bounded values.
    pub fn new<K, V>(flags: impl IntoIterator<Item = (K, V)>) -> Result<Self, MountError>
    where
        K: Into<String>,
        V: Into<String>,
    {
        let mut values = BTreeMap::new();
        for (name, value) in flags {
            if values.len() >= HARD_MAX_FLAGS {
                return Err(MountError::new(MountErrorKind::MetadataTooLarge));
            }
            let name = name.into();
            let value = value.into();
            let name_valid = !name.is_empty()
                && name.len() <= MAX_FLAG_NAME_BYTES
                && name.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'-' | b'_')
                });
            let value_valid = value.len() <= MAX_FLAG_VALUE_BYTES
                && value.chars().all(|character| !character.is_control());
            if !name_valid || !value_valid || values.insert(name, value).is_some() {
                return Err(MountError::new(MountErrorKind::MetadataTooLarge));
            }
        }
        Ok(Self(values))
    }

    /// Returns the number of inert flags.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns whether no inert flags are declared.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }
}

/// Complete input for one identity-bound initial component mount.
pub struct PrivateMountRequest {
    pub(crate) key: DocumentMountKey,
    pub(crate) parameters: CanonicalValue,
    pub(crate) flags: MountFlags,
    pub(crate) document_path: Option<crate::snapshot::MountedDocumentPath>,
}

impl PrivateMountRequest {
    /// Groups validated document identity, typed parameters, and inert flags.
    #[must_use]
    pub const fn new(key: DocumentMountKey, parameters: CanonicalValue, flags: MountFlags) -> Self {
        Self {
            key,
            parameters,
            flags,
            document_path: None,
        }
    }

    /// Seals the already-matched document path into the initial signed snapshot.
    #[must_use]
    pub fn with_document_path(
        mut self,
        document_path: crate::snapshot::MountedDocumentPath,
    ) -> Self {
        self.document_path = Some(document_path);
        self
    }
}

impl fmt::Debug for PrivateMountRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<PrivateMountRequest:redacted>")
    }
}

/// Browser-publishable private island returned only after ledger authority exists.
pub struct PrivateMountOutput {
    pub(crate) body: String,
    pub(crate) metadata: MountMetadata,
    pub(crate) instance_id: InstanceId,
    pub(crate) revision: Revision,
    pub(crate) expires_at: UnixMillis,
}

impl PrivateMountOutput {
    /// Returns complete engine-owned island HTML.
    #[must_use]
    pub fn body(&self) -> &[u8] {
        self.body.as_bytes()
    }

    /// Returns inert typed document mount metadata.
    #[must_use]
    pub const fn metadata(&self) -> &MountMetadata {
        &self.metadata
    }

    /// Returns the server-assigned identity whose ledger authority now exists.
    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    /// Returns the initial authoritative revision.
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Returns the exclusive authority deadline.
    #[must_use]
    pub const fn expires_at(&self) -> UnixMillis {
        self.expires_at
    }

    /// Consumes the completed mount into checked document markup and inert metadata.
    #[must_use]
    pub fn into_document_parts(self) -> (TrustedHtml, MountMetadata) {
        (
            TrustedHtml::engine_validated_island(self.body),
            self.metadata,
        )
    }
}

impl fmt::Debug for PrivateMountOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PrivateMountOutput")
            .field("body_bytes", &self.body.len())
            .field("metadata", &self.metadata)
            .field("revision", &self.revision)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}
