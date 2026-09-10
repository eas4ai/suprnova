//! Versioned Complete and Composite entry codec with structural integrity
//! and body-free inspection.

use std::collections::BTreeMap;

use bytes::Bytes;
use sha2::{Digest as _, Sha256};

use super::composite::{CompositeEntry, CompositeHeader, MAX_STITCH_SLOTS};
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
    /// Segment graph requiring assembly.
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

/// Whether `value` is a valid HTTP header value byte for byte: horizontal
/// tab, the visible ASCII range `0x20` through `0x7e`, and the obsolete text
/// range `0x80` and above. Everything else is refused, which covers CR, LF,
/// and NUL - the bytes that would let a value smuggle a second header or
/// truncate the wire representation - along with every other control byte.
///
/// This is exactly `http::HeaderValue`'s own rule, and it has to be: a
/// stored value is always formable as an `http::HeaderValue`, so the
/// response builders in [`super::hot`] never fail on their own stored
/// values. A looser rule here would let a value be stored that no hit can
/// ever be served from - every request would miss, render, and republish the
/// same unformable value, forever.
///
/// Shared by [`SafeHeaders::from_pairs`] and the Composite nonce-header
/// template check, which apply the same rule to a different value shape (a
/// stored value versus an assembled piece of one).
pub(super) fn header_value_is_safe(value: &str) -> bool {
    value
        .bytes()
        .all(|b| b == b'\t' || (0x20..=0x7e).contains(&b) || b >= 0x80)
}

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
            if value.len() > 4_096 || !header_value_is_safe(value) {
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

/// A decoded stored representation of either kind.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecodedEntry {
    /// Directly sendable final bytes.
    Complete(CompleteEntry),
    /// A segment graph that needs assembly.
    Composite(CompositeEntry),
}

impl DecodedEntry {
    /// The shared header.
    #[must_use]
    pub fn header(&self) -> &EntryHeader {
        match self {
            Self::Complete(entry) => entry.header(),
            Self::Composite(entry) => entry.header(),
        }
    }

    /// The kind.
    #[must_use]
    pub const fn kind(&self) -> EntryKind {
        match self {
            Self::Complete(_) => EntryKind::Complete,
            Self::Composite(_) => EntryKind::Composite,
        }
    }

    /// The Complete entry, if that is what this is.
    #[must_use]
    pub fn into_complete(self) -> Option<CompleteEntry> {
        match self {
            Self::Complete(entry) => Some(entry),
            Self::Composite(_) => None,
        }
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
    /// Stitch slots (0 for a Complete entry).
    pub slots: usize,
}

fn integrity(keys: &SnapshotKeyRing, bytes: &[u8]) -> [u8; 32] {
    keys.mac(crate::crypto::SnapshotPurpose::RenderEntryV1, &[bytes])
        .expect("active key derives")
}

/// Bounds for the entry header's canonical JSON form: the caller's header
/// byte ceiling with the engine's fixed structural bounds (container depth,
/// entry count, string length) underneath it. Fallible because the byte
/// ceiling comes from the caller-supplied [`EntryLimits`], not a fixed
/// constant, and [`crate::limits::InputLimits::new`] rejects a zero or
/// above-ceiling value.
///
/// `pub(super)` so [`super::composite::CompositeHeader::canonical_bytes`]
/// shares exactly this bound instead of repeating the tuple: a Composite
/// header's canonical framing can never diverge from a Complete header's.
pub(super) fn header_limits(
    max_header_bytes: usize,
) -> Result<crate::limits::InputLimits, RenderCacheError> {
    crate::limits::InputLimits::new(max_header_bytes, 32, 512, 4_096)
        .map_err(|_| RenderCacheError::new(RenderCacheErrorKind::EntryInvalid))
}

/// Validates that `header_bytes` is well-formed canonical-bounded JSON
/// (byte length, container depth, entry count, and string length) before it
/// is handed to `serde_json`. Encoding runs the header through the same
/// bounds via [`crate::canonical::parse_canonical_value`] and
/// [`crate::canonical::to_canonical_bytes`], so decoding re-checks them here
/// rather than trusting stored bytes: an oversized or too-deeply-nested
/// header fails closed on both sides.
fn validate_header_bounds(
    header_bytes: &[u8],
    limits: &crate::limits::InputLimits,
) -> Result<(), RenderCacheError> {
    crate::canonical::parse_canonical_value(header_bytes, limits)
        .map(|_| ())
        .map_err(|_| RenderCacheError::new(RenderCacheErrorKind::EntryInvalid))
}

/// Frames already-canonicalized header bytes and a body into the wire
/// layout (magic, version, kind, lengths, header, body, digest, integrity
/// tag) and signs it. Shared by [`encode`], [`encode_composite`],
/// and the test-only raw header encoder below, since all three produce the
/// same wire shape from different header sources.
fn frame(
    header_bytes: &[u8],
    body: &[u8],
    kind: EntryKind,
    keys: &SnapshotKeyRing,
) -> Result<Bytes, RenderCacheError> {
    let invalid = || RenderCacheError::new(RenderCacheErrorKind::EntryInvalid);
    let header_len = u32::try_from(header_bytes.len()).map_err(|_| invalid())?;
    let body_len = u32::try_from(body.len()).map_err(|_| invalid())?;
    let mut out = Vec::with_capacity(4 + 2 + 1 + 4 + header_bytes.len() + 4 + body.len() + 64);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&ENTRY_FORMAT_VERSION.to_be_bytes());
    out.push(match kind {
        EntryKind::Complete => 1,
        EntryKind::Composite => 2,
    });
    out.extend_from_slice(&header_len.to_be_bytes());
    out.extend_from_slice(header_bytes);
    out.extend_from_slice(&body_len.to_be_bytes());
    out.extend_from_slice(body);
    let digest: [u8; 32] = Sha256::digest(body).into();
    out.extend_from_slice(&digest);
    let mac = integrity(keys, &out);
    out.extend_from_slice(&mac);
    Ok(Bytes::from(out))
}

/// Encodes a Complete entry: its canonical header framed around exactly the
/// body bytes it was constructed with.
pub fn encode(entry: &CompleteEntry, keys: &SnapshotKeyRing) -> Result<Bytes, RenderCacheError> {
    let invalid = || RenderCacheError::new(RenderCacheErrorKind::EntryInvalid);
    let header_json = serde_json::to_vec(entry.header()).map_err(|_| invalid())?;
    let limits = header_limits(EntryLimits::default().max_header_bytes)?;
    let header_bytes = crate::canonical::parse_canonical_value(&header_json, &limits)
        .and_then(|value| crate::canonical::to_canonical_bytes(&value, &limits))
        .map_err(|_| invalid())?;
    frame(&header_bytes, entry.body(), EntryKind::Complete, keys)
}

/// Encodes a Composite entry: its canonical header (the shared entry header
/// plus the segment graph) framed around the shell bytes, never a slot's
/// rendered content.
pub fn encode_composite(
    entry: &CompositeEntry,
    keys: &SnapshotKeyRing,
) -> Result<Bytes, RenderCacheError> {
    let header_bytes = entry.canonical_header_bytes(EntryLimits::default().max_header_bytes)?;
    frame(&header_bytes, entry.shell(), EntryKind::Composite, keys)
}

/// Test-only encoder that serializes an arbitrary `Serialize` value as the
/// header instead of a typed [`EntryHeader`], so tests can produce a
/// structurally valid, correctly-signed entry whose header carries content
/// the typed constructors (`SafeHeaders::from_pairs`,
/// `VarianceDescriptor::declare`) would have rejected. `decode` must still
/// reject such an entry: an intact integrity tag alone is not sufficient,
/// since a future encoder bug or another producer sharing the key ring
/// could otherwise smuggle unsafe content through.
#[doc(hidden)]
#[must_use]
pub fn encode_raw_header_for_test<T: serde::Serialize>(
    header: &T,
    body: &Bytes,
    keys: &SnapshotKeyRing,
) -> Bytes {
    encode_raw_header_for_test_with_kind(header, body, keys, EntryKind::Complete)
}

/// [`encode_raw_header_for_test`] with an explicit kind byte, so a test can
/// produce a structurally valid, correctly-signed entry of either kind
/// whose header carries content the typed constructors would have
/// rejected. Used to prove that a Composite header disagreeing with its
/// shell is rejected by `decode`, not merely by [`CompositeEntry::new`].
#[doc(hidden)]
#[must_use]
pub fn encode_raw_header_for_test_with_kind<T: serde::Serialize>(
    header: &T,
    body: &Bytes,
    keys: &SnapshotKeyRing,
    kind: EntryKind,
) -> Bytes {
    let header_json = serde_json::to_vec(header).expect("test header serializes");
    let limits =
        header_limits(EntryLimits::default().max_header_bytes).expect("default header limits");
    let header_bytes = crate::canonical::parse_canonical_value(&header_json, &limits)
        .and_then(|value| crate::canonical::to_canonical_bytes(&value, &limits))
        .expect("test header canonicalizes");
    frame(&header_bytes, body, kind, keys).expect("test framing")
}

fn read_u32(bytes: &[u8], at: usize) -> Result<usize, RenderCacheError> {
    let slice = bytes
        .get(at..at + 4)
        .ok_or_else(|| RenderCacheError::new(RenderCacheErrorKind::EntryInvalid))?;
    Ok(u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]) as usize)
}

/// Rebuilds a decoded [`EntryHeader`] through its validating constructors
/// and re-checks the header, observation, and status bounds. The derived
/// `Deserialize` for `SafeHeaders` and `VarianceDescriptor` rebuilds their
/// private maps straight from JSON, bypassing the allowlist, charset,
/// count, and length bounds their validating constructors apply. A correct
/// integrity tag only proves the bytes were not corrupted in transit, not
/// that whatever produced them respected those bounds, so what was read
/// back is rebuilt through the same validating constructors before it is
/// trusted. Shared by the Complete and Composite decode arms, since both
/// carry the same [`EntryHeader`] shape.
fn rebuild_header(
    header: EntryHeader,
    limits: &EntryLimits,
) -> Result<EntryHeader, RenderCacheError> {
    let invalid = || RenderCacheError::new(RenderCacheErrorKind::EntryInvalid);
    if header.headers.0.len() > limits.max_headers
        || header.observed.len() > limits.max_observations
        || header.status != 200
    {
        return Err(invalid());
    }
    let rebuilt_headers = SafeHeaders::from_pairs(header.headers.iter()).map_err(|_| invalid())?;
    let mut rebuilt_variance = VarianceDescriptor::new();
    for (dimension, value) in header.variance.dimensions() {
        rebuilt_variance
            .declare(dimension.clone(), value.clone())
            .map_err(|_| invalid())?;
    }
    Ok(EntryHeader {
        headers: rebuilt_headers,
        variance: rebuilt_variance,
        ..header
    })
}

/// Decodes and verifies a Complete or Composite entry; every defect is a miss.
pub fn decode(
    bytes: &Bytes,
    keys: &SnapshotKeyRing,
    limits: &EntryLimits,
) -> Result<DecodedEntry, RenderCacheError> {
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
        2 => EntryKind::Composite,
        _ => return Err(invalid()),
    };
    let header_len = read_u32(bytes, 7)?;
    if header_len > limits.max_header_bytes {
        return Err(invalid());
    }
    let header_start = 11;
    let header_bytes = bytes
        .get(header_start..header_start + header_len)
        .ok_or_else(invalid)?;
    let canonical_limits = header_limits(limits.max_header_bytes)?;
    validate_header_bounds(header_bytes, &canonical_limits)?;
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
    match kind {
        EntryKind::Complete => {
            let header: EntryHeader =
                serde_json::from_slice(header_bytes).map_err(|_| invalid())?;
            let header = rebuild_header(header, limits)?;
            let entry = CompleteEntry::new(header, body);
            if entry.validator() != &Validator::Strong(stored_digest) {
                return Err(invalid());
            }
            Ok(DecodedEntry::Complete(entry))
        }
        EntryKind::Composite => {
            let composite: CompositeHeader =
                serde_json::from_slice(header_bytes).map_err(|_| invalid())?;
            let header = rebuild_header(composite.entry, limits)?;
            // A Composite entry is only ever produced for the shell-stitched
            // class; a header claiming any other class is not a defect the
            // integrity tag could ever have caught (a valid tag only proves
            // the bytes are unaltered, not that whatever produced them
            // respected this contract), so it fails closed here rather than
            // being trusted downstream as, say, an unassembled public-shared
            // entry.
            if header.class != RepresentationClass::PublicShellStitched {
                return Err(invalid());
            }
            if Validator::strong_for(&body) != Validator::Strong(stored_digest) {
                return Err(invalid());
            }
            let entry = CompositeEntry::new(header, composite.graph, body)?;
            Ok(DecodedEntry::Composite(entry))
        }
    }
}

/// Reads metadata without decoding or exposing the body. This is the
/// lighter read: it verifies structural bounds and the integrity of the
/// framing (magic, version, kind, lengths, and the stitch-slot ceiling on a
/// Composite graph) but, unlike `decode`, it neither takes a key ring nor
/// applies the business-rule bounds `decode` enforces on the header content
/// (the safe-header allowlist, the variance bounds, or the stored digest).
/// The asymmetry is deliberate, since `inspect` exists for body-free triage
/// without those inputs. What it never does is report an unbounded number
/// read out of unauthenticated bytes: every field of [`EntryInspection`] is
/// either bounded by the framing it was read through or refused.
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
    let canonical_limits = header_limits(limits.max_header_bytes)?;
    validate_header_bounds(header_bytes, &canonical_limits)?;
    let header: EntryHeader = serde_json::from_slice(header_bytes).map_err(|_| invalid())?;
    let body_bytes = read_u32(bytes, 11 + header_len)?;
    // The same bound `decode` puts on this field, for the same reason: the
    // length is four unauthenticated bytes, and a length above the ceiling
    // belongs to an entry no read can ever serve.
    if body_bytes > limits.max_body_bytes {
        return Err(invalid());
    }
    let slots = match kind {
        EntryKind::Complete => 0,
        EntryKind::Composite => {
            let composite: CompositeHeader =
                serde_json::from_slice(header_bytes).map_err(|_| invalid())?;
            // The graph arrives straight from `serde_json` here, with
            // neither an integrity check nor `SegmentGraph::validate` in the
            // way, so its slot vector is bounded only by the canonical
            // entry count (512) and not by the stitch bound. Reporting that
            // number would state as fact a slot count `decode` refuses and
            // no assembly could ever serve, so a graph over the bound is
            // refused here too: `inspect` and `decode` give the same verdict
            // on this bound, and `slots` is never a number above it.
            let slots = composite.graph.slots.len();
            if slots > MAX_STITCH_SLOTS {
                return Err(invalid());
            }
            slots
        }
    };
    Ok(EntryInspection {
        kind,
        class: header.class,
        body_bytes,
        status: header.status,
        published_at_ms: header.published_at_ms,
        epoch: header.epoch,
        observations: header.observed.len(),
        slots,
    })
}

/// Base64url text (de)serialization for [`RenderKey`], reused by any typed
/// field elsewhere in this crate that names a key without embedding key
/// material in the wire form.
pub(crate) mod render_key_serde {
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

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use http::header::HeaderValue;

    use super::{
        CompleteEntry, EntryHeader, EntryLimits, RenderCacheErrorKind, RepresentationClass,
        SafeHeaders, decode, encode, header_value_is_safe, inspect, integrity,
    };
    use crate::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
    use crate::identity::{KeyId, UnixMillis};
    use crate::render_cache::generation::GenerationSet;
    use crate::render_cache::key::RenderKey;
    use crate::render_cache::variance::VarianceDescriptor;

    fn keys() -> SnapshotKeyRing {
        let active = KeyRecord::new(
            KeyId::parse("entry-codec-test").expect("key id"),
            RootKey::new(vec![7; 32]).expect("root key"),
            UnixMillis::new(0),
            UnixMillis::new(u64::MAX / 2),
            UnixMillis::new(u64::MAX),
        )
        .expect("key record");
        SnapshotKeyRing::new(active, Vec::new()).expect("key ring")
    }

    fn complete_entry(keys: &SnapshotKeyRing) -> CompleteEntry {
        CompleteEntry::new(
            EntryHeader {
                key: RenderKey::for_test(keys, "/kind-byte"),
                class: RepresentationClass::PublicShared,
                variance: VarianceDescriptor::new(),
                published_at_ms: 1_000,
                fresh_ms: 60_000,
                stale_servable_ms: 0,
                stale_on_error_ms: 0,
                observed: GenerationSet::default(),
                epoch: 1,
                seed_deadline_ms: None,
                status: 200,
                headers: SafeHeaders::from_pairs([("content-type", "text/html; charset=utf-8")])
                    .expect("safe headers"),
                content_encoding: None,
            },
            Bytes::from_static(b"<!doctype html><html><body>hi</body></html>"),
        )
    }

    /// The format defines exactly two kind bytes. Any other byte is refused
    /// by both readers, and refused *as the kind byte*: the integrity tag is
    /// recomputed over the patched frame, so these bytes are correctly
    /// signed and the kind check is the only thing left that can reject
    /// them.
    ///
    /// Recomputing the tag is what makes this a test. `decode` verifies the
    /// tag before it ever reads byte six, so a test that merely patched the
    /// byte would pass against a `decode` with no kind check at all. Only
    /// `inspect`, which takes no key ring, reads the byte unconditionally.
    #[test]
    fn a_kind_byte_the_format_does_not_define_is_refused_by_both_readers() {
        let keys = keys();
        let encoded = encode(&complete_entry(&keys), &keys).expect("encode");
        assert_eq!(encoded[6], 1, "a Complete entry frames kind byte 1");
        for kind_byte in [0_u8, 3, 4, 127, 255] {
            let mut patched = encoded.to_vec();
            patched[6] = kind_byte;
            let signed_len = patched.len() - 32;
            let mac = integrity(&keys, &patched[..signed_len]);
            patched[signed_len..].copy_from_slice(&mac);
            let patched = Bytes::from(patched);
            assert_eq!(
                decode(&patched, &keys, &EntryLimits::default())
                    .map(|_| ())
                    .map_err(|error| error.kind()),
                Err(RenderCacheErrorKind::EntryInvalid),
                "decode accepted kind byte {kind_byte}"
            );
            assert_eq!(
                inspect(&patched, &EntryLimits::default())
                    .map(|_| ())
                    .map_err(|error| error.kind()),
                Err(RenderCacheErrorKind::EntryInvalid),
                "inspect accepted kind byte {kind_byte}"
            );
        }
    }

    /// The positive control the check above needs: the kind byte the format
    /// does define is still accepted by both readers, so "refuses
    /// everything" cannot pass for "refuses what it must". Kind byte 2 is
    /// covered end to end by the Composite round trip in
    /// `tests/render_cache_entry.rs`.
    #[test]
    fn the_defined_kind_byte_is_still_accepted() {
        let keys = keys();
        let encoded = encode(&complete_entry(&keys), &keys).expect("encode");
        assert!(
            decode(&encoded, &keys, &EntryLimits::default()).is_ok(),
            "an unpatched Complete entry decodes"
        );
        assert_eq!(
            inspect(&encoded, &EntryLimits::default())
                .expect("an unpatched Complete entry inspects")
                .kind,
            super::EntryKind::Complete
        );
    }

    /// The rule this module applies to a stored header value has to be the
    /// rule the response builders in [`super::super::hot`] can form, or a
    /// value can be stored that no hit can ever be served from - a miss that
    /// renders and republishes the same unformable value on every request.
    ///
    /// Every byte, against `http`'s own answer. A byte below `0x80` is its
    /// own one-byte UTF-8 character; `0x80` through `0xff` become the two
    /// byte encoding of `U+0080` through `U+00ff`, whose bytes are all at or
    /// above `0x80`, so the high half is covered by the values that can
    /// actually occur in a `&str`.
    #[test]
    fn the_stored_value_rule_is_http_header_value_validity_byte_for_byte() {
        for byte in 0_u8..=u8::MAX {
            let text = char::from(byte).to_string();
            let encoded = text.as_bytes();
            assert_eq!(
                header_value_is_safe(&text),
                HeaderValue::from_bytes(encoded).is_ok(),
                "the two rules disagree for byte {byte:#04x}, encoded {encoded:02x?}"
            );
        }
    }

    /// The three bytes the old rule named are still refused, stated
    /// separately so a future widening of the rule above cannot quietly let
    /// a header-smuggling byte back in.
    #[test]
    fn the_header_smuggling_bytes_are_still_refused() {
        for byte in [b'\r', b'\n', 0] {
            let text = char::from(byte).to_string();
            assert!(
                !header_value_is_safe(&text),
                "byte {byte:#04x} must never be storable in a header value"
            );
        }
    }
}
