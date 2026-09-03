//! Versioned Complete entry codec with structural integrity and body-free
//! inspection.

use std::collections::BTreeMap;

use bytes::Bytes;
use sha2::{Digest as _, Sha256};

use super::generation::GenerationSet;
use super::key::RenderKey;
use super::policy::{RepresentationClass, UNSAFE_RESPONSE_HEADERS};
use super::variance::VarianceDescriptor;
use super::{RenderCacheError, RenderCacheErrorKind};
use crate::crypto::SnapshotKeyRing;

/// Entry format version.
pub const ENTRY_FORMAT_VERSION: u16 = 1;
const MAGIC: &[u8; 4] = b"SNRC";

/// Decoding bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EntryLimits {
    /// Largest body accepted.
    pub max_body_bytes: usize,
    /// Largest header JSON accepted.
    pub max_header_bytes: usize,
    /// Most safe headers accepted.
    pub max_headers: usize,
    /// Most dependency observations accepted.
    pub max_observations: usize,
}

impl Default for EntryLimits {
    fn default() -> Self {
        Self {
            max_body_bytes: 8 * 1024 * 1024,
            max_header_bytes: 64 * 1024,
            max_headers: 32,
            max_observations: 4_096,
        }
    }
}

/// Representation form.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntryKind {
    /// Directly sendable final bytes.
    Complete,
    /// Segment graph requiring assembly; unsupported in this build.
    Composite,
}

/// Headers allowed to replay from storage: an allowlist that excludes
/// hop-by-hop, connection, cookie, and per-request tracing headers.
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SafeHeaders(BTreeMap<String, String>);

/// Header names a stored representation may carry.
pub const REPLAYABLE_HEADERS: [&str; 8] = [
    "cache-control",
    "content-language",
    "content-security-policy",
    "content-type",
    "link",
    "referrer-policy",
    "vary",
    "x-content-type-options",
];

impl SafeHeaders {
    /// Builds from lower-cased pairs; any name outside the allowlist fails.
    pub fn from_pairs<I, K, V>(pairs: I) -> Result<Self, RenderCacheError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut map = BTreeMap::new();
        for (name, value) in pairs {
            let name = name.as_ref().to_ascii_lowercase();
            if UNSAFE_RESPONSE_HEADERS.contains(&name.as_str())
                || !REPLAYABLE_HEADERS.contains(&name.as_str())
            {
                return Err(RenderCacheError::new(RenderCacheErrorKind::EntryInvalid));
            }
            let value = value.as_ref();
            if value.len() > 4_096 || value.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0) {
                return Err(RenderCacheError::new(RenderCacheErrorKind::EntryInvalid));
            }
            map.insert(name, value.to_owned());
        }
        Ok(Self(map))
    }

    /// The pairs in canonical order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

/// A represented-byte validator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Validator {
    /// SHA-256 of the exact final bytes.
    Strong([u8; 32]),
}

impl Validator {
    /// Strong validator for exact bytes.
    #[must_use]
    pub fn strong_for(body: &[u8]) -> Self {
        Self::Strong(Sha256::digest(body).into())
    }

    /// The digest as base64url without padding.
    #[must_use]
    pub fn digest_base64url(&self) -> String {
        use base64::Engine as _;
        let Self::Strong(digest) = self;
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
    }

    /// The quoted strong `ETag` value.
    #[must_use]
    pub fn etag(&self) -> String {
        format!("\"sha256-{}\"", self.digest_base64url())
    }
}

/// Everything about a Complete entry except its body.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EntryHeader {
    /// Lookup key.
    #[serde(with = "render_key_serde")]
    pub key: RenderKey,
    /// Effective class after classification.
    pub class: RepresentationClass,
    /// Declared variance.
    pub variance: VarianceDescriptor,
    /// Publication time in Unix milliseconds.
    pub published_at_ms: u64,
    /// Fresh interval.
    pub fresh_ms: u64,
    /// Stale-servable interval.
    pub stale_servable_ms: u64,
    /// Stale-on-error interval.
    pub stale_on_error_ms: u64,
    /// Dependency generations observed at render.
    pub observed: GenerationSet,
    /// Authority epoch at render.
    pub epoch: u64,
    /// Earliest public seed promotion deadline embedded in the body, if any.
    pub seed_deadline_ms: Option<u64>,
    /// Final status; only 200 today.
    pub status: u16,
    /// Replayable headers.
    pub headers: SafeHeaders,
    /// Content encoding of the body bytes.
    pub content_encoding: Option<String>,
}

/// A directly sendable representation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompleteEntry {
    header: EntryHeader,
    body: Bytes,
    validator: Validator,
}

impl CompleteEntry {
    /// Binds a header to exact body bytes and computes the validator.
    #[must_use]
    pub fn new(header: EntryHeader, body: Bytes) -> Self {
        let validator = Validator::strong_for(&body);
        Self {
            header,
            body,
            validator,
        }
    }

    /// Header.
    #[must_use]
    pub fn header(&self) -> &EntryHeader {
        &self.header
    }

    /// Shared immutable body bytes; cloning shares, never copies.
    #[must_use]
    pub fn body(&self) -> &Bytes {
        &self.body
    }

    /// Validator over exactly `body`.
    #[must_use]
    pub fn validator(&self) -> &Validator {
        &self.validator
    }
}

/// Body-free metadata read from encoded bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntryInspection {
    /// Form.
    pub kind: EntryKind,
    /// Class.
    pub class: RepresentationClass,
    /// Body length in bytes.
    pub body_bytes: usize,
    /// Status.
    pub status: u16,
    /// Publication time.
    pub published_at_ms: u64,
    /// Epoch.
    pub epoch: u64,
    /// Number of observed dependencies.
    pub observations: usize,
}

fn integrity(keys: &SnapshotKeyRing, bytes: &[u8]) -> [u8; 32] {
    keys.mac(crate::crypto::SnapshotPurpose::RenderEntryV1, &[bytes])
        .expect("active key derives")
}

/// Bounds for the entry header's canonical JSON form.
fn header_limits() -> crate::limits::InputLimits {
    crate::limits::InputLimits::new(EntryLimits::default().max_header_bytes, 32, 512, 4_096)
        .expect("header limits are within the engine ceilings")
}

/// Validates that `header_bytes` is well-formed canonical-bounded JSON
/// (byte length, container depth, entry count, and string length) before it
/// is handed to `serde_json`. Encoding runs the header through the same
/// bounds via [`crate::canonical::parse_canonical_value`] and
/// [`crate::canonical::to_canonical_bytes`], so decoding re-checks them here
/// rather than trusting stored bytes: an oversized or too-deeply-nested
/// header fails closed on both sides.
fn validate_header_bounds(header_bytes: &[u8]) -> Result<(), RenderCacheError> {
    let limits = header_limits();
    crate::canonical::parse_canonical_value(header_bytes, &limits)
        .map(|_| ())
        .map_err(|_| RenderCacheError::new(RenderCacheErrorKind::EntryInvalid))
}

fn encode_with_kind(
    entry: &CompleteEntry,
    keys: &SnapshotKeyRing,
    kind: EntryKind,
) -> Result<Bytes, RenderCacheError> {
    let invalid = || RenderCacheError::new(RenderCacheErrorKind::EntryInvalid);
    let header = serde_json::to_vec(entry.header()).map_err(|_| invalid())?;
    let limits = header_limits();
    let header = crate::canonical::parse_canonical_value(&header, &limits)
        .and_then(|value| crate::canonical::to_canonical_bytes(&value, &limits))
        .map_err(|_| invalid())?;
    let mut out = Vec::with_capacity(4 + 2 + 1 + 4 + header.len() + 4 + entry.body().len() + 64);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&ENTRY_FORMAT_VERSION.to_be_bytes());
    out.push(match kind {
        EntryKind::Complete => 1,
        EntryKind::Composite => 2,
    });
    out.extend_from_slice(&(header.len() as u32).to_be_bytes());
    out.extend_from_slice(&header);
    out.extend_from_slice(&(entry.body().len() as u32).to_be_bytes());
    out.extend_from_slice(entry.body());
    let Validator::Strong(digest) = entry.validator();
    out.extend_from_slice(digest);
    let mac = integrity(keys, &out);
    out.extend_from_slice(&mac);
    Ok(Bytes::from(out))
}

/// Encodes a Complete entry.
pub fn encode(entry: &CompleteEntry, keys: &SnapshotKeyRing) -> Result<Bytes, RenderCacheError> {
    encode_with_kind(entry, keys, EntryKind::Complete)
}

/// Test-only encoder that forces the stored kind byte, so tests can produce
/// a structurally valid but unsupported entry (for example `Composite`)
/// without a second, unaudited encoding path in production code.
#[doc(hidden)]
#[must_use]
pub fn encode_with_kind_for_test(
    entry: &CompleteEntry,
    keys: &SnapshotKeyRing,
    kind: EntryKind,
) -> Bytes {
    encode_with_kind(entry, keys, kind).expect("test encoding")
}

fn read_u32(bytes: &[u8], at: usize) -> Result<usize, RenderCacheError> {
    let slice = bytes
        .get(at..at + 4)
        .ok_or_else(|| RenderCacheError::new(RenderCacheErrorKind::EntryInvalid))?;
    Ok(u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]) as usize)
}

/// Decodes and verifies a Complete entry; every defect is a miss.
pub fn decode(
    bytes: &Bytes,
    keys: &SnapshotKeyRing,
    limits: &EntryLimits,
) -> Result<CompleteEntry, RenderCacheError> {
    let invalid = || RenderCacheError::new(RenderCacheErrorKind::EntryInvalid);
    if bytes.len() < 4 + 2 + 1 + 4 + 4 + 32 + 32 || &bytes[..4] != MAGIC {
        return Err(invalid());
    }
    let (payload, mac) = bytes.split_at(bytes.len() - 32);
    if integrity(keys, payload) != mac {
        return Err(invalid());
    }
    let version = u16::from_be_bytes([bytes[4], bytes[5]]);
    if version != ENTRY_FORMAT_VERSION {
        return Err(RenderCacheError::new(
            RenderCacheErrorKind::EntryUnsupported,
        ));
    }
    let kind = match bytes[6] {
        1 => EntryKind::Complete,
        2 => {
            return Err(RenderCacheError::new(
                RenderCacheErrorKind::EntryUnsupported,
            ));
        }
        _ => return Err(invalid()),
    };
    debug_assert_eq!(kind, EntryKind::Complete);
    let header_len = read_u32(bytes, 7)?;
    if header_len > limits.max_header_bytes {
        return Err(invalid());
    }
    let header_start = 11;
    let header_bytes = bytes
        .get(header_start..header_start + header_len)
        .ok_or_else(invalid)?;
    validate_header_bounds(header_bytes)?;
    let header: EntryHeader = serde_json::from_slice(header_bytes).map_err(|_| invalid())?;
    if header.headers.0.len() > limits.max_headers
        || header.observed.len() > limits.max_observations
        || header.status != 200
    {
        return Err(invalid());
    }
    let body_len_at = header_start + header_len;
    let body_len = read_u32(bytes, body_len_at)?;
    if body_len > limits.max_body_bytes {
        return Err(invalid());
    }
    let body_start = body_len_at + 4;
    // `Bytes::slice` panics when the range runs past the buffer; a corrupted
    // `body_len` can be within `max_body_bytes` yet still overrun the actual
    // encoded bytes, so the end is bounds-checked before it is ever sliced.
    let digest_at = body_start.checked_add(body_len).ok_or_else(invalid)?;
    if digest_at > bytes.len() {
        return Err(invalid());
    }
    let body = bytes.slice(body_start..digest_at);
    let stored_digest: [u8; 32] = bytes
        .get(digest_at..digest_at + 32)
        .ok_or_else(invalid)?
        .try_into()
        .map_err(|_| invalid())?;
    if digest_at + 32 != payload.len() {
        return Err(invalid());
    }
    let entry = CompleteEntry::new(header, body);
    if entry.validator() != &Validator::Strong(stored_digest) {
        return Err(invalid());
    }
    Ok(entry)
}

/// Reads metadata without decoding or exposing the body.
pub fn inspect(bytes: &Bytes, limits: &EntryLimits) -> Result<EntryInspection, RenderCacheError> {
    let invalid = || RenderCacheError::new(RenderCacheErrorKind::EntryInvalid);
    if bytes.len() < 11 || &bytes[..4] != MAGIC {
        return Err(invalid());
    }
    let kind = match bytes[6] {
        1 => EntryKind::Complete,
        2 => EntryKind::Composite,
        _ => return Err(invalid()),
    };
    let header_len = read_u32(bytes, 7)?;
    if header_len > limits.max_header_bytes {
        return Err(invalid());
    }
    let header_bytes = bytes.get(11..11 + header_len).ok_or_else(invalid)?;
    validate_header_bounds(header_bytes)?;
    let header: EntryHeader = serde_json::from_slice(header_bytes).map_err(|_| invalid())?;
    let body_bytes = read_u32(bytes, 11 + header_len)?;
    Ok(EntryInspection {
        kind,
        class: header.class,
        body_bytes,
        status: header.status,
        published_at_ms: header.published_at_ms,
        epoch: header.epoch,
        observations: header.observed.len(),
    })
}

mod render_key_serde {
    use serde::{Deserialize as _, Deserializer, Serializer};

    use super::RenderKey;

    pub fn serialize<S: Serializer>(key: &RenderKey, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&key.to_base64url())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<RenderKey, D::Error> {
        let text = String::deserialize(deserializer)?;
        RenderKey::from_base64url(&text).map_err(serde::de::Error::custom)
    }
}
