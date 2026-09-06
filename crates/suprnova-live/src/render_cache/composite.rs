//! Composite entries: a typed, bounded segment graph over reusable shell
//! bytes with stitch slots for identity-bound islands, and (in
//! [`assemble`]) the deterministic request-time assembler that turns one
//! graph plus current-request slot outcomes into final bytes.
//!
//! A Composite entry never contains an island that depends on who asked;
//! those islands are re-rendered by the host on every hit and dropped into
//! typed slots here. The graph carries what each slot needs to be re-mounted
//! (route, slot, document key, component, contract, protocol, build,
//! canonical parameters, inert flags) and what to do if that fails.

use std::collections::{BTreeMap, BTreeSet};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use bytes::Bytes;
use sha2::{Digest as _, Sha256};

use super::entry::{EntryHeader, REPLAYABLE_HEADERS, SafeHeaders, Validator};
use super::{RenderCacheError, RenderCacheErrorKind};
use crate::canonical::CanonicalValue;
use crate::identity::{BuildId, ComponentName, ContentDigest, IslandSlot, RouteIdentity};
use crate::mount::{DocumentMountKey, MountFlags};
use crate::view::TrustedHtml;

/// Most identity-bound slots one Composite graph may declare.
pub const MAX_STITCH_SLOTS: usize = 32;
/// Most nonce holes in one shell.
pub const MAX_NONCE_HOLES: usize = 64;
/// Most segments in one graph: a literal around every slot and hole, plus one.
pub const MAX_SEGMENTS: usize = 2 * MAX_STITCH_SLOTS + MAX_NONCE_HOLES + 1;
/// Largest canonical parameter document one slot may carry, in bytes.
pub const MAX_SLOT_PARAMETER_BYTES: usize = 4_096;
/// Largest declared fallback fragment, in bytes (the canonical header string bound).
pub const MAX_FALLBACK_BYTES: usize = 4_096;
/// Most public-seed islands recorded as remaining inside the shell.
pub const MAX_SHELL_ISLANDS: usize = 128;
/// Most nonce-bearing header templates.
pub const MAX_NONCE_HEADERS: usize = 4;
/// Bytes of shell on each side of a slot that its surrounding digest covers.
pub const SURROUNDING_WINDOW_BYTES: usize = 64;
/// Longest accepted nonce, in bytes.
pub const MAX_NONCE_BYTES: usize = 256;
/// Largest assembled header value, in bytes: the same bound
/// [`super::entry::SafeHeaders::from_pairs`] applies to a stored header
/// value, applied here to the value a nonce-header template assembles into.
const MAX_HEADER_VALUE_BYTES: usize = 4_096;

fn invalid() -> RenderCacheError {
    RenderCacheError::new(RenderCacheErrorKind::EntryInvalid)
}

/// One ordered piece of the assembled body.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Segment {
    /// The next `len` bytes of the shell.
    Literal {
        /// Byte length.
        len: u32,
    },
    /// The output of `slots[index]`.
    Slot {
        /// Index into [`SegmentGraph::slots`].
        index: u16,
    },
    /// The fresh nonce generated at assembly.
    Nonce,
}

impl Segment {
    /// The literal length, if this is a literal segment.
    #[must_use]
    pub const fn literal_len(&self) -> Option<u32> {
        match self {
            Self::Literal { len } => Some(*len),
            Self::Slot { .. } | Self::Nonce => None,
        }
    }
}

/// What assembly does when a slot's island cannot be rendered for this request.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SlotFailurePolicy {
    /// The whole document fails; the host falls back to its uncached render path.
    FailDocument,
    /// The island is left out.
    Omit,
    /// A declared framework-typed fragment takes the island's place.
    Fallback {
        /// Trusted fallback markup, at most [`MAX_FALLBACK_BYTES`].
        html: String,
    },
}

/// One typed hole for one identity-bound island.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StitchSlot {
    /// Route identity, base64url.
    pub route: String,
    /// Island slot name.
    pub slot: String,
    /// Server-declared document mount key.
    pub document_key: String,
    /// Component name.
    pub component: String,
    /// Component contract digest, base64url.
    pub contract_digest: String,
    /// Protocol version the mount was declared with.
    pub protocol: u16,
    /// Build identity the declaration belongs to.
    pub build: String,
    /// RFC 8785 canonical JSON of the mount parameters.
    pub parameters: String,
    /// Inert mount flags.
    pub flags: BTreeMap<String, String>,
    /// Declared failure behavior.
    pub on_failure: SlotFailurePolicy,
    /// SHA-256 (base64url) of the shell bytes around this slot; see [`surrounding_digest`].
    pub surrounding: String,
}

/// A [`StitchSlot`] with every identity parsed into its typed form.
#[derive(Clone, Debug)]
pub struct ParsedSlot {
    /// Route identity.
    pub route: RouteIdentity,
    /// Island slot.
    pub slot: IslandSlot,
    /// Document mount key.
    pub document_key: DocumentMountKey,
    /// Component name.
    pub component: ComponentName,
    /// Contract digest.
    pub contract_digest: ContentDigest,
    /// Protocol version.
    pub protocol: u16,
    /// Build identity.
    pub build: BuildId,
    /// Mount parameters.
    pub parameters: CanonicalValue,
    /// Inert mount flags.
    pub flags: MountFlags,
    /// Declared failure behavior.
    pub on_failure: SlotFailurePolicy,
}

impl StitchSlot {
    /// Parses every identity and bound; any defect is `EntryInvalid`.
    pub fn parse(&self) -> Result<ParsedSlot, RenderCacheError> {
        if self.parameters.len() > MAX_SLOT_PARAMETER_BYTES {
            return Err(invalid());
        }
        let limits = crate::limits::InputLimits::new(
            MAX_SLOT_PARAMETER_BYTES,
            32,
            512,
            MAX_SLOT_PARAMETER_BYTES,
        )
        .map_err(|_| invalid())?;
        let parameters =
            crate::canonical::parse_canonical_value(self.parameters.as_bytes(), &limits)
                .map_err(|_| invalid())?;
        let canonical =
            crate::canonical::to_canonical_bytes(&parameters, &limits).map_err(|_| invalid())?;
        if canonical != self.parameters.as_bytes() {
            return Err(invalid());
        }
        if !crate::SUPPORTED_PROTOCOL_VERSIONS.contains(&self.protocol) {
            return Err(invalid());
        }
        if let SlotFailurePolicy::Fallback { html } = &self.on_failure
            && html.len() > MAX_FALLBACK_BYTES
        {
            return Err(invalid());
        }
        decode_digest(&self.surrounding)?;
        Ok(ParsedSlot {
            route: RouteIdentity::parse(&self.route).map_err(|_| invalid())?,
            slot: IslandSlot::parse(&self.slot).map_err(|_| invalid())?,
            document_key: DocumentMountKey::parse(&self.document_key).map_err(|_| invalid())?,
            component: ComponentName::parse(&self.component).map_err(|_| invalid())?,
            contract_digest: ContentDigest::parse(&self.contract_digest).map_err(|_| invalid())?,
            protocol: self.protocol,
            build: BuildId::parse(&self.build).map_err(|_| invalid())?,
            parameters,
            flags: MountFlags::new(self.flags.iter().map(|(k, v)| (k.clone(), v.clone())))
                .map_err(|_| invalid())?,
            on_failure: self.on_failure.clone(),
        })
    }
}

/// A public-seed island that stays inside the shell.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ShellIsland {
    /// Island slot name.
    pub slot: String,
    /// Document mount key.
    pub document_key: String,
}

/// One piece of a nonce-bearing header value.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum HeaderPiece {
    /// Literal header text.
    Text {
        /// The text.
        text: String,
    },
    /// The fresh nonce.
    Nonce,
}

/// A replayable header whose value carries the nonce.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HeaderTemplate {
    /// Lower-case header name from [`REPLAYABLE_HEADERS`].
    pub name: String,
    /// Ordered pieces; at least one is [`HeaderPiece::Nonce`].
    pub pieces: Vec<HeaderPiece>,
}

/// The typed segment graph of a Composite entry.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SegmentGraph {
    /// Ordered segments; literal lengths partition the shell exactly.
    pub segments: Vec<Segment>,
    /// Slots in segment order.
    pub slots: Vec<StitchSlot>,
    /// Public-seed islands remaining inside the shell.
    pub shell_islands: Vec<ShellIsland>,
    /// Header templates that carry the nonce.
    pub nonce_headers: Vec<HeaderTemplate>,
}

impl SegmentGraph {
    /// Whether assembly must supply a nonce.
    #[must_use]
    pub fn needs_nonce(&self) -> bool {
        self.segments
            .iter()
            .any(|segment| matches!(segment, Segment::Nonce))
            || !self.nonce_headers.is_empty()
    }

    /// Validates every structural rule and bound against a shell of `shell_len` bytes.
    pub fn validate(&self, shell_len: usize) -> Result<(), RenderCacheError> {
        if self.segments.len() > MAX_SEGMENTS
            || self.slots.len() > MAX_STITCH_SLOTS
            || self.shell_islands.len() > MAX_SHELL_ISLANDS
            || self.nonce_headers.len() > MAX_NONCE_HEADERS
        {
            return Err(invalid());
        }
        let mut literal_total: usize = 0;
        let mut holes = 0usize;
        let mut next_slot = 0usize;
        for segment in &self.segments {
            match segment {
                Segment::Literal { len } => {
                    literal_total = literal_total
                        .checked_add(*len as usize)
                        .ok_or_else(invalid)?;
                }
                Segment::Slot { index } => {
                    if usize::from(*index) != next_slot {
                        return Err(invalid());
                    }
                    next_slot += 1;
                }
                Segment::Nonce => holes += 1,
            }
        }
        if literal_total != shell_len || next_slot != self.slots.len() || holes > MAX_NONCE_HOLES {
            return Err(invalid());
        }
        let mut slots_seen = BTreeSet::new();
        let mut keys_seen = BTreeSet::new();
        for island in &self.shell_islands {
            IslandSlot::parse(&island.slot).map_err(|_| invalid())?;
            DocumentMountKey::parse(&island.document_key).map_err(|_| invalid())?;
            if !slots_seen.insert(island.slot.as_str())
                || !keys_seen.insert(island.document_key.as_str())
            {
                return Err(invalid());
            }
        }
        for slot in &self.slots {
            slot.parse()?;
            if !slots_seen.insert(slot.slot.as_str())
                || !keys_seen.insert(slot.document_key.as_str())
            {
                return Err(invalid());
            }
        }
        let mut header_names = BTreeSet::new();
        for template in &self.nonce_headers {
            if !REPLAYABLE_HEADERS.contains(&template.name.as_str())
                || !header_names.insert(template.name.as_str())
                || template.pieces.len() > MAX_NONCE_HOLES
                || !template
                    .pieces
                    .iter()
                    .any(|piece| matches!(piece, HeaderPiece::Nonce))
            {
                return Err(invalid());
            }
            // A `Nonce` piece is not yet a nonce (assembly has not run), but
            // the budget must hold for whatever nonce actually lands there,
            // so it counts the worst case, `MAX_NONCE_BYTES`, rather than 0.
            let text_len: usize = template
                .pieces
                .iter()
                .map(|piece| match piece {
                    HeaderPiece::Text { text } => text.len(),
                    HeaderPiece::Nonce => MAX_NONCE_BYTES,
                })
                .sum();
            if text_len > MAX_HEADER_VALUE_BYTES
                || template.pieces.iter().any(|piece| match piece {
                    HeaderPiece::Text { text } => !super::entry::header_value_is_safe(text),
                    HeaderPiece::Nonce => false,
                })
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

/// SHA-256 (base64url) over the shell bytes adjacent to slot `index`: a
/// 4-byte big-endian length followed by the last [`SURROUNDING_WINDOW_BYTES`]
/// of the literal segment immediately before the slot (empty when the
/// previous segment is not a literal), then a 4-byte big-endian length
/// followed by the first [`SURROUNDING_WINDOW_BYTES`] of the literal segment
/// immediately after it (empty likewise). The length prefixes frame the two
/// windows injectively; a fixed zero-byte separator would not, since either
/// window may itself contain a zero byte. Computed from the graph and the
/// shell alone, so the assembler can recompute it without the original
/// document.
pub fn surrounding_digest(
    graph: &SegmentGraph,
    shell: &[u8],
    index: usize,
) -> Result<String, RenderCacheError> {
    let mut cursor = 0usize;
    let mut literal_ranges: Vec<Option<(usize, usize)>> = Vec::with_capacity(graph.segments.len());
    for segment in &graph.segments {
        match segment {
            Segment::Literal { len } => {
                let end = cursor.checked_add(*len as usize).ok_or_else(invalid)?;
                if end > shell.len() {
                    return Err(invalid());
                }
                literal_ranges.push(Some((cursor, end)));
                cursor = end;
            }
            Segment::Slot { .. } | Segment::Nonce => literal_ranges.push(None),
        }
    }
    let position = graph
        .segments
        .iter()
        .position(
            |segment| matches!(segment, Segment::Slot { index: i } if usize::from(*i) == index),
        )
        .ok_or_else(invalid)?;
    let before = position
        .checked_sub(1)
        .and_then(|p| literal_ranges[p])
        .map(|(start, end)| &shell[end.saturating_sub(SURROUNDING_WINDOW_BYTES).max(start)..end])
        .unwrap_or(&[]);
    let after = literal_ranges
        .get(position + 1)
        .copied()
        .flatten()
        .map(|(start, end)| &shell[start..(start + SURROUNDING_WINDOW_BYTES).min(end)])
        .unwrap_or(&[]);
    let mut hasher = Sha256::new();
    hasher.update((before.len() as u32).to_be_bytes());
    hasher.update(before);
    hasher.update((after.len() as u32).to_be_bytes());
    hasher.update(after);
    Ok(URL_SAFE_NO_PAD.encode(hasher.finalize()))
}

/// Whether `text` is syntactically a digest: canonical unpadded base64url
/// decoding to exactly 32 bytes. Its only caller checks format, not content,
/// so it reports success or failure rather than handing back bytes nothing
/// uses.
fn decode_digest(text: &str) -> Result<(), RenderCacheError> {
    let bytes = URL_SAFE_NO_PAD.decode(text).map_err(|_| invalid())?;
    if bytes.len() == 32 {
        Ok(())
    } else {
        Err(invalid())
    }
}

/// The stored header of a Composite entry: every Complete header field plus the graph.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CompositeHeader {
    /// The shared entry header.
    #[serde(flatten)]
    pub entry: EntryHeader,
    /// The segment graph.
    pub graph: SegmentGraph,
}

impl CompositeHeader {
    /// Canonical bounded JSON bytes of this header, as the codec writes them.
    pub fn canonical_bytes(&self, max_header_bytes: usize) -> Result<Vec<u8>, RenderCacheError> {
        let json = serde_json::to_vec(self).map_err(|_| invalid())?;
        let limits = super::entry::header_limits(max_header_bytes)?;
        crate::canonical::parse_canonical_value(&json, &limits)
            .and_then(|value| crate::canonical::to_canonical_bytes(&value, &limits))
            .map_err(|_| invalid())
    }
}

/// A representation that needs assembly before it can be sent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompositeEntry {
    header: EntryHeader,
    graph: SegmentGraph,
    shell: Bytes,
    structural: [u8; 32],
}

impl CompositeEntry {
    /// Validates the graph against the shell and binds header, graph, and shell into a structural digest.
    ///
    /// This checks the graph's own structure (bounds, slot order, literal
    /// partitioning, identity syntax) against `shell`'s length; it does not
    /// recompute each slot's [`surrounding`](StitchSlot::surrounding) digest
    /// against `shell`'s actual bytes; that comparison belongs to the
    /// request-time assembler, which is the only place a drifted shell is
    /// meaningfully observable. The structural digest is computed over the
    /// canonical header bytes under `EntryLimits::default()`'s
    /// `max_header_bytes`; that default is the contract, and the codec
    /// encodes under the same default, so the two can never diverge.
    pub fn new(
        header: EntryHeader,
        graph: SegmentGraph,
        shell: Bytes,
    ) -> Result<Self, RenderCacheError> {
        graph.validate(shell.len())?;
        let canonical = CompositeHeader {
            entry: header.clone(),
            graph: graph.clone(),
        }
        .canonical_bytes(super::entry::EntryLimits::default().max_header_bytes)?;
        let mut hasher = Sha256::new();
        hasher.update(&canonical);
        hasher.update(&shell);
        Ok(Self {
            header,
            graph,
            shell,
            structural: hasher.finalize().into(),
        })
    }

    /// Header.
    #[must_use]
    pub fn header(&self) -> &EntryHeader {
        &self.header
    }

    /// Segment graph.
    #[must_use]
    pub fn graph(&self) -> &SegmentGraph {
        &self.graph
    }

    /// Shared shell bytes; cloning shares, never copies.
    #[must_use]
    pub fn shell(&self) -> &Bytes {
        &self.shell
    }

    /// Structural digest over the canonical header and the shell. Never an HTTP validator.
    #[must_use]
    pub fn structural_digest(&self) -> &[u8; 32] {
        &self.structural
    }

    /// Whether assembly must supply a nonce.
    #[must_use]
    pub fn needs_nonce(&self) -> bool {
        self.graph.needs_nonce()
    }

    /// The canonical header bytes the codec frames.
    pub fn canonical_header_bytes(
        &self,
        max_header_bytes: usize,
    ) -> Result<Vec<u8>, RenderCacheError> {
        CompositeHeader {
            entry: self.header.clone(),
            graph: self.graph.clone(),
        }
        .canonical_bytes(max_header_bytes)
    }
}

/// Whether `value` is an acceptable nonce: 1 to [`MAX_NONCE_BYTES`] bytes of `[A-Za-z0-9+/=_-]`.
#[must_use]
pub fn valid_nonce(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_NONCE_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=' | b'-' | b'_')
        })
}

/// A fresh 128-bit nonce as unpadded base64url (22 characters).
pub fn fresh_nonce() -> Result<String, RenderCacheError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|_| RenderCacheError::new(RenderCacheErrorKind::ProviderUnavailable))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// An island the host rendered and validated for this request.
#[derive(Debug)]
pub struct CheckedIsland {
    html: TrustedHtml,
    slot: IslandSlot,
    document_key: DocumentMountKey,
}

impl CheckedIsland {
    /// Binds validated island markup to the identity it was rendered for.
    #[must_use]
    pub const fn new(html: TrustedHtml, slot: IslandSlot, document_key: DocumentMountKey) -> Self {
        Self {
            html,
            slot,
            document_key,
        }
    }
}

/// The outcome of one slot for one request.
#[derive(Debug)]
pub enum SlotOutcome {
    /// The island rendered under current authority.
    Rendered(CheckedIsland),
    /// The slot's declared fallback fragment is used.
    Fallback,
    /// The slot's declared omit behavior is used.
    Omitted,
}

/// Everything assembly needs beyond the entry.
#[derive(Debug)]
pub struct AssemblyInput {
    /// One outcome per slot, in slot order.
    pub outcomes: Vec<SlotOutcome>,
    /// The fresh nonce, present exactly when the graph needs one.
    pub nonce: Option<String>,
}

/// A fully assembled representation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssembledDocument {
    body: Bytes,
    validator: Validator,
    headers: SafeHeaders,
}

impl AssembledDocument {
    /// Final bytes.
    #[must_use]
    pub fn body(&self) -> &Bytes {
        &self.body
    }

    /// Strong validator over exactly `body`.
    #[must_use]
    pub fn validator(&self) -> &Validator {
        &self.validator
    }

    /// Replayable headers with every nonce template rendered.
    #[must_use]
    pub fn headers(&self) -> &SafeHeaders {
        &self.headers
    }
}

fn assembly_failed() -> RenderCacheError {
    RenderCacheError::new(RenderCacheErrorKind::AssemblyFailed)
}

/// Turns one graph plus current-request outcomes into final bytes and headers.
/// Pure and deterministic; every rejection is [`RenderCacheErrorKind::AssemblyFailed`].
pub fn assemble(
    entry: &CompositeEntry,
    input: AssemblyInput,
    max_body_bytes: usize,
) -> Result<AssembledDocument, RenderCacheError> {
    let graph = entry.graph();
    if input.outcomes.len() != graph.slots.len() {
        return Err(assembly_failed());
    }
    let nonce = match (&input.nonce, entry.needs_nonce()) {
        (Some(nonce), true) if valid_nonce(nonce) => Some(nonce.as_str()),
        (None, false) => None,
        _ => return Err(assembly_failed()),
    };
    let mut slots_seen: BTreeSet<&str> = graph
        .shell_islands
        .iter()
        .map(|island| island.slot.as_str())
        .collect();
    let mut keys_seen: BTreeSet<&str> = graph
        .shell_islands
        .iter()
        .map(|island| island.document_key.as_str())
        .collect();
    for (slot, outcome) in graph.slots.iter().zip(&input.outcomes) {
        match outcome {
            SlotOutcome::Rendered(island) => {
                if island.slot.as_str() != slot.slot
                    || island.document_key.as_str() != slot.document_key
                {
                    return Err(assembly_failed());
                }
                if !slots_seen.insert(island.slot.as_str())
                    || !keys_seen.insert(island.document_key.as_str())
                {
                    return Err(assembly_failed());
                }
            }
            SlotOutcome::Fallback => {
                if !matches!(slot.on_failure, SlotFailurePolicy::Fallback { .. }) {
                    return Err(assembly_failed());
                }
            }
            SlotOutcome::Omitted => {
                if !matches!(slot.on_failure, SlotFailurePolicy::Omit) {
                    return Err(assembly_failed());
                }
            }
        }
    }
    for index in 0..graph.slots.len() {
        if surrounding_digest(graph, entry.shell(), index)? != graph.slots[index].surrounding {
            return Err(assembly_failed());
        }
    }
    let shell = entry.shell();
    let mut body: Vec<u8> = Vec::with_capacity(shell.len() + 4_096);
    let mut cursor = 0usize;
    let push = |body: &mut Vec<u8>, bytes: &[u8]| -> Result<(), RenderCacheError> {
        body.extend_from_slice(bytes);
        if body.len() > max_body_bytes {
            Err(assembly_failed())
        } else {
            Ok(())
        }
    };
    for segment in &graph.segments {
        match segment {
            Segment::Literal { len } => {
                let end = cursor + *len as usize;
                push(&mut body, &shell[cursor..end])?;
                cursor = end;
            }
            Segment::Slot { index } => match &input.outcomes[usize::from(*index)] {
                SlotOutcome::Rendered(island) => push(&mut body, island.html.as_str().as_bytes())?,
                SlotOutcome::Fallback => match &graph.slots[usize::from(*index)].on_failure {
                    SlotFailurePolicy::Fallback { html } => push(&mut body, html.as_bytes())?,
                    SlotFailurePolicy::FailDocument | SlotFailurePolicy::Omit => {
                        return Err(assembly_failed());
                    }
                },
                SlotOutcome::Omitted => {}
            },
            Segment::Nonce => push(&mut body, nonce.ok_or_else(assembly_failed)?.as_bytes())?,
        }
    }
    let mut pairs: BTreeMap<String, String> = entry
        .header()
        .headers
        .iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    for template in &graph.nonce_headers {
        let mut value = String::new();
        for piece in &template.pieces {
            match piece {
                HeaderPiece::Text { text } => value.push_str(text),
                HeaderPiece::Nonce => value.push_str(nonce.ok_or_else(assembly_failed)?),
            }
        }
        pairs.insert(template.name.clone(), value);
    }
    let headers = SafeHeaders::from_pairs(pairs).map_err(|_| assembly_failed())?;
    let validator = Validator::strong_for(&body);
    Ok(AssembledDocument {
        body: Bytes::from(body),
        validator,
        headers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
    use crate::identity::{KeyId, UnixMillis};
    use crate::render_cache::RepresentationClass;
    use crate::render_cache::entry::SafeHeaders;
    use crate::render_cache::generation::GenerationSet;
    use crate::render_cache::key::RenderKey;
    use crate::render_cache::variance::VarianceDescriptor;

    pub(super) fn keys() -> SnapshotKeyRing {
        let active = KeyRecord::new(
            KeyId::parse("composite-test").expect("key id"),
            RootKey::new(vec![5; 32]).expect("root key"),
            UnixMillis::new(0),
            UnixMillis::new(u64::MAX / 2),
            UnixMillis::new(u64::MAX),
        )
        .expect("key record");
        SnapshotKeyRing::new(active, Vec::new()).expect("key ring")
    }

    pub(super) fn header(keys: &SnapshotKeyRing) -> EntryHeader {
        EntryHeader {
            key: RenderKey::for_test(keys, "/stitched"),
            class: RepresentationClass::PublicShellStitched,
            variance: VarianceDescriptor::new(),
            published_at_ms: 1_000,
            fresh_ms: 60_000,
            stale_servable_ms: 0,
            stale_on_error_ms: 0,
            observed: GenerationSet::default(),
            epoch: 1,
            seed_deadline_ms: None,
            status: 200,
            headers: SafeHeaders::from_pairs([
                ("content-type", "text/html; charset=utf-8"),
                ("content-security-policy", "script-src 'nonce-OLDNONCE'"),
            ])
            .expect("safe headers"),
            content_encoding: None,
        }
    }

    pub(super) fn slot(name: &str, key: &str, on_failure: SlotFailurePolicy) -> StitchSlot {
        StitchSlot {
            route: RouteIdentity::from_bytes(&[9u8; 32])
                .expect("route")
                .to_base64url(),
            slot: name.to_owned(),
            document_key: key.to_owned(),
            component: "app.counter".to_owned(),
            contract_digest: ContentDigest::from_bytes(&[1u8; 32])
                .expect("digest")
                .to_base64url(),
            protocol: 1,
            build: "suprnova-1.0.0".to_owned(),
            parameters: "{}".to_owned(),
            flags: BTreeMap::new(),
            on_failure,
            surrounding: String::new(),
        }
    }

    /// `<!doctype html><html><body><p>head</p>[slot a][NONCE]<p>mid</p>[slot b]</body></html>`
    pub(super) fn graph_and_shell() -> (SegmentGraph, Bytes) {
        let head = b"<!doctype html><html><body><p>head</p>".to_vec();
        let mid = b"<p>mid</p>".to_vec();
        let tail = b"</body></html>".to_vec();
        let shell = Bytes::from([head.as_slice(), mid.as_slice(), tail.as_slice()].concat());
        let mut graph = SegmentGraph {
            segments: vec![
                Segment::Literal {
                    len: head.len() as u32,
                },
                Segment::Slot { index: 0 },
                Segment::Nonce,
                Segment::Literal {
                    len: mid.len() as u32,
                },
                Segment::Slot { index: 1 },
                Segment::Literal {
                    len: tail.len() as u32,
                },
            ],
            slots: vec![
                slot("a", "doc-a", SlotFailurePolicy::FailDocument),
                slot("b", "doc-b", SlotFailurePolicy::Omit),
            ],
            shell_islands: vec![ShellIsland {
                slot: "seed".to_owned(),
                document_key: "doc-seed".to_owned(),
            }],
            nonce_headers: vec![HeaderTemplate {
                name: "content-security-policy".to_owned(),
                pieces: vec![
                    HeaderPiece::Text {
                        text: "script-src 'nonce-".to_owned(),
                    },
                    HeaderPiece::Nonce,
                    HeaderPiece::Text {
                        text: "'".to_owned(),
                    },
                ],
            }],
        };
        for index in 0..graph.slots.len() {
            graph.slots[index].surrounding =
                surrounding_digest(&graph, &shell, index).expect("surrounding");
        }
        (graph, shell)
    }

    #[test]
    fn a_well_formed_graph_validates_and_needs_a_nonce() {
        let (graph, shell) = graph_and_shell();
        graph.validate(shell.len()).expect("valid graph");
        assert!(graph.needs_nonce());
    }

    #[test]
    fn a_graph_without_holes_or_templates_needs_no_nonce() {
        let (mut graph, shell) = graph_and_shell();
        graph
            .segments
            .retain(|segment| !matches!(segment, Segment::Nonce));
        graph.nonce_headers.clear();
        for index in 0..graph.slots.len() {
            graph.slots[index].surrounding =
                surrounding_digest(&graph, &shell, index).expect("surrounding");
        }
        graph.validate(shell.len()).expect("valid graph");
        assert!(!graph.needs_nonce());
    }

    #[test]
    fn literal_lengths_must_partition_the_shell_exactly() {
        let (graph, shell) = graph_and_shell();
        assert_eq!(
            graph.validate(shell.len() + 1).map_err(|e| e.kind()),
            Err(RenderCacheErrorKind::EntryInvalid)
        );
        assert_eq!(
            graph.validate(shell.len() - 1).map_err(|e| e.kind()),
            Err(RenderCacheErrorKind::EntryInvalid)
        );
    }

    #[test]
    fn slot_indexes_must_each_appear_once_in_order_and_in_range() {
        let (mut graph, shell) = graph_and_shell();
        graph.segments[1] = Segment::Slot { index: 1 };
        graph.segments[4] = Segment::Slot { index: 0 };
        assert!(graph.validate(shell.len()).is_err(), "out of order");
        let (mut graph, shell) = graph_and_shell();
        graph.segments[4] = Segment::Slot { index: 0 };
        assert!(
            graph.validate(shell.len()).is_err(),
            "duplicate index, one slot unused"
        );
        let (mut graph, shell) = graph_and_shell();
        graph.segments[4] = Segment::Slot { index: 7 };
        assert!(graph.validate(shell.len()).is_err(), "out of range");
    }

    #[test]
    fn island_identities_are_unique_across_slots_and_shell_islands() {
        let (mut graph, shell) = graph_and_shell();
        graph.shell_islands.push(ShellIsland {
            slot: "a".to_owned(),
            document_key: "other".to_owned(),
        });
        assert!(graph.validate(shell.len()).is_err(), "duplicate slot name");
        let (mut graph, shell) = graph_and_shell();
        graph.shell_islands.push(ShellIsland {
            slot: "other".to_owned(),
            document_key: "doc-b".to_owned(),
        });
        assert!(
            graph.validate(shell.len()).is_err(),
            "duplicate document key"
        );
    }

    #[test]
    fn every_bound_is_enforced() {
        let (mut graph, shell) = graph_and_shell();
        graph.slots[0].parameters = "x".repeat(MAX_SLOT_PARAMETER_BYTES + 1);
        assert!(graph.validate(shell.len()).is_err(), "parameter bytes");
        let (mut graph, shell) = graph_and_shell();
        graph.slots[0].on_failure = SlotFailurePolicy::Fallback {
            html: "x".repeat(MAX_FALLBACK_BYTES + 1),
        };
        assert!(graph.validate(shell.len()).is_err(), "fallback bytes");
        let (mut graph, shell) = graph_and_shell();
        graph.slots[0].parameters = "not json".to_owned();
        assert!(
            graph.validate(shell.len()).is_err(),
            "parameters must be canonical JSON"
        );
        let (mut graph, shell) = graph_and_shell();
        graph.slots[0].protocol = 99;
        assert!(graph.validate(shell.len()).is_err(), "unsupported protocol");
        let (mut graph, shell) = graph_and_shell();
        graph.nonce_headers[0].name = "set-cookie".to_owned();
        assert!(
            graph.validate(shell.len()).is_err(),
            "non-replayable header"
        );
        let (mut graph, shell) = graph_and_shell();
        graph.nonce_headers[0].pieces = vec![HeaderPiece::Text {
            text: "no hole".to_owned(),
        }];
        assert!(
            graph.validate(shell.len()).is_err(),
            "template without a nonce piece"
        );
        let (mut graph, shell) = graph_and_shell();
        // 17 Nonce pieces carry no text of their own, but each must be
        // budgeted at MAX_NONCE_BYTES for the value the assembled nonce will
        // actually occupy: 17 * 256 = 4,352 > MAX_HEADER_VALUE_BYTES (4,096).
        graph.nonce_headers[0].pieces = vec![HeaderPiece::Nonce; 17];
        assert!(
            graph.validate(shell.len()).is_err(),
            "nonce-only template exceeds the assembled header value budget"
        );
        let (mut graph, shell) = graph_and_shell();
        graph.nonce_headers[0].pieces = vec![HeaderPiece::Nonce; MAX_NONCE_HOLES + 1];
        assert!(
            graph.validate(shell.len()).is_err(),
            "too many pieces in one header template"
        );
        let (mut graph, shell) = graph_and_shell();
        for _ in 0..MAX_NONCE_HOLES {
            graph.segments.insert(2, Segment::Nonce);
        }
        assert!(graph.validate(shell.len()).is_err(), "too many nonce holes");
        let (mut graph, shell) = graph_and_shell();
        for i in 0..MAX_SHELL_ISLANDS {
            graph.shell_islands.push(ShellIsland {
                slot: format!("s{i}"),
                document_key: format!("k{i}"),
            });
        }
        assert!(
            graph.validate(shell.len()).is_err(),
            "too many shell islands"
        );
    }

    #[test]
    fn too_many_slots_is_invalid() {
        let (mut graph, shell) = graph_and_shell();
        let mut segments = vec![Segment::Literal {
            len: shell.len() as u32,
        }];
        graph.slots.clear();
        for i in 0..=MAX_STITCH_SLOTS {
            graph.slots.push(slot(
                &format!("s{i}"),
                &format!("k{i}"),
                SlotFailurePolicy::Omit,
            ));
            segments.push(Segment::Slot { index: i as u16 });
        }
        graph.segments = segments;
        assert!(graph.validate(shell.len()).is_err());
    }

    #[test]
    fn too_many_segments_is_invalid() {
        // `validate` checks `segments.len() > MAX_SEGMENTS` as part of one
        // combined bound check that runs before the per-segment loop that
        // counts nonce holes, so this graph would also fail the nonce-hole
        // bound on its own; the segments-count check fires first either way,
        // and that is the branch this test reaches.
        let graph = SegmentGraph {
            segments: vec![Segment::Nonce; MAX_SEGMENTS + 1],
            slots: Vec::new(),
            shell_islands: Vec::new(),
            nonce_headers: Vec::new(),
        };
        assert!(graph.validate(0).is_err());
    }

    #[test]
    fn too_many_nonce_headers_is_invalid() {
        let (mut graph, shell) = graph_and_shell();
        for _ in 0..MAX_NONCE_HEADERS {
            graph.nonce_headers.push(HeaderTemplate {
                name: "vary".to_owned(),
                pieces: vec![HeaderPiece::Nonce],
            });
        }
        assert!(graph.validate(shell.len()).is_err());
    }

    #[test]
    fn a_composite_entry_binds_header_graph_and_shell_into_a_structural_digest() {
        let keys = keys();
        let (graph, shell) = graph_and_shell();
        let entry =
            CompositeEntry::new(header(&keys), graph.clone(), shell.clone()).expect("entry");
        let same = CompositeEntry::new(header(&keys), graph.clone(), shell.clone()).expect("entry");
        assert_eq!(entry.structural_digest(), same.structural_digest());
        let mut other_shell = shell.to_vec();
        other_shell[0] ^= 1;
        let other = CompositeEntry::new(header(&keys), graph.clone(), Bytes::from(other_shell))
            .expect("entry");
        assert_ne!(entry.structural_digest(), other.structural_digest());
        assert_ne!(
            *entry.structural_digest(),
            <[u8; 32]>::from(sha2::Sha256::digest(&shell)),
            "the structural digest is not a body digest"
        );
        let mut renamed_graph = graph.clone();
        renamed_graph.slots[0].slot = "renamed".to_owned();
        let renamed =
            CompositeEntry::new(header(&keys), renamed_graph, shell.clone()).expect("entry");
        assert_ne!(
            entry.structural_digest(),
            renamed.structural_digest(),
            "the structural digest binds every StitchSlot field, not just the shell bytes"
        );
    }

    #[test]
    fn the_composite_header_round_trips_through_json_and_keeps_entry_fields_readable() {
        let keys = keys();
        let (graph, _) = graph_and_shell();
        let composite = CompositeHeader {
            entry: header(&keys),
            graph,
        };
        let json = serde_json::to_vec(&composite).expect("serializes");
        let back: CompositeHeader = serde_json::from_slice(&json).expect("deserializes");
        assert_eq!(back, composite);
        let entry_only: EntryHeader = serde_json::from_slice(&json).expect("entry fields readable");
        assert_eq!(entry_only.class, RepresentationClass::PublicShellStitched);
        let canonical = composite.canonical_bytes(64 * 1024).expect("canonical");
        assert!(canonical.starts_with(b"{\""));
    }

    #[test]
    fn parsed_slots_expose_typed_identities() {
        let (graph, _) = graph_and_shell();
        let parsed = graph.slots[0].parse().expect("parses");
        assert_eq!(parsed.slot.as_str(), "a");
        assert_eq!(parsed.document_key.as_str(), "doc-a");
        assert_eq!(parsed.component.as_str(), "app.counter");
        assert_eq!(parsed.protocol, 1);
        assert_eq!(
            parsed.parameters,
            crate::canonical::CanonicalValue::Object(BTreeMap::new())
        );
    }

    #[test]
    fn fresh_nonces_match_the_grammar_and_differ() {
        let first = fresh_nonce().expect("nonce");
        let second = fresh_nonce().expect("nonce");
        assert!(valid_nonce(&first));
        assert_eq!(first.len(), 22);
        assert_ne!(first, second);
        assert!(!valid_nonce(""));
        assert!(!valid_nonce("has space"));
        assert!(valid_nonce(&"a".repeat(MAX_NONCE_BYTES)));
        assert!(!valid_nonce(&"a".repeat(257)));
    }

    #[test]
    fn surrounding_digest_at_the_first_segment_has_an_empty_before_window() {
        let shell = b"0123456789".to_vec();
        let graph = SegmentGraph {
            segments: vec![
                Segment::Slot { index: 0 },
                Segment::Literal {
                    len: shell.len() as u32,
                },
            ],
            slots: Vec::new(),
            shell_islands: Vec::new(),
            nonce_headers: Vec::new(),
        };
        let digest = surrounding_digest(&graph, &shell, 0).expect("computed");
        let mut changed_shell = shell.clone();
        changed_shell[0] ^= 1;
        let changed = surrounding_digest(&graph, &changed_shell, 0).expect("computed");
        assert_ne!(
            digest, changed,
            "the after-window is the only content, so changing it must change the digest"
        );
    }

    #[test]
    fn surrounding_digest_at_the_last_segment_has_an_empty_after_window() {
        let shell = b"0123456789".to_vec();
        let graph = SegmentGraph {
            segments: vec![
                Segment::Literal {
                    len: shell.len() as u32,
                },
                Segment::Slot { index: 0 },
            ],
            slots: Vec::new(),
            shell_islands: Vec::new(),
            nonce_headers: Vec::new(),
        };
        assert!(surrounding_digest(&graph, &shell, 0).is_ok());
    }

    #[test]
    fn surrounding_digest_of_adjacent_slots_never_reads_past_its_own_literal() {
        let a = b"aaaaaaaaaa".to_vec();
        let b = b"bbbbbbbbbb".to_vec();
        let c = b"cccccccccc".to_vec();
        let shell = Bytes::from([a.as_slice(), b.as_slice(), c.as_slice()].concat());
        let graph = SegmentGraph {
            segments: vec![
                Segment::Literal {
                    len: a.len() as u32,
                },
                Segment::Slot { index: 0 },
                Segment::Literal {
                    len: b.len() as u32,
                },
                Segment::Slot { index: 1 },
                Segment::Literal {
                    len: c.len() as u32,
                },
            ],
            slots: Vec::new(),
            shell_islands: Vec::new(),
            nonce_headers: Vec::new(),
        };
        let digest_0 = surrounding_digest(&graph, &shell, 0).expect("computed");
        let digest_1 = surrounding_digest(&graph, &shell, 1).expect("computed");

        // Slot 0's window is `a` (before) and `b` (after); it never reads
        // `c`, so flipping a byte there must not move its digest.
        let mut shell_with_changed_c = shell.to_vec();
        *shell_with_changed_c.last_mut().expect("non-empty") ^= 1;
        let digest_0_after_c_changed =
            surrounding_digest(&graph, &shell_with_changed_c, 0).expect("computed");
        assert_eq!(digest_0, digest_0_after_c_changed, "slot 0 must not read c");

        // Slot 1's window is `b` (before) and `c` (after); it never reads
        // `a`, so flipping a byte there must not move its digest.
        let mut shell_with_changed_a = shell.to_vec();
        shell_with_changed_a[0] ^= 1;
        let digest_1_after_a_changed =
            surrounding_digest(&graph, &shell_with_changed_a, 1).expect("computed");
        assert_eq!(digest_1, digest_1_after_a_changed, "slot 1 must not read a");
    }

    fn island(name: &str, key: &str, body: &str) -> CheckedIsland {
        let html = TrustedHtml::framework_generated(
            format!("<div data-suprnova-live-root=\"{name}\" data-suprnova-live-document-key=\"{key}\">{body}</div>"),
            crate::view::TrustedMarkupReason::new("composite test island").expect("reason"),
        )
        .expect("trusted");
        CheckedIsland::new(
            html,
            IslandSlot::parse(name).expect("slot"),
            DocumentMountKey::parse(key).expect("key"),
        )
    }

    fn entry() -> CompositeEntry {
        let keys = keys();
        let (graph, shell) = graph_and_shell();
        CompositeEntry::new(header(&keys), graph, shell).expect("entry")
    }

    #[test]
    fn assembly_is_deterministic_and_fills_every_hole() {
        let entry = entry();
        let input = || AssemblyInput {
            outcomes: vec![
                SlotOutcome::Rendered(island("a", "doc-a", "A")),
                SlotOutcome::Omitted,
            ],
            nonce: Some("abcDEF123-_".to_owned()),
        };
        let first = assemble(&entry, input(), 1 << 20).expect("assembles");
        let second = assemble(&entry, input(), 1 << 20).expect("assembles");
        assert_eq!(first.body(), second.body());
        assert_eq!(first.validator(), second.validator());
        let body = std::str::from_utf8(first.body()).expect("utf8");
        assert_eq!(
            body,
            "<!doctype html><html><body><p>head</p><div data-suprnova-live-root=\"a\" data-suprnova-live-document-key=\"doc-a\">A</div>abcDEF123-_<p>mid</p></body></html>"
        );
        assert_eq!(first.validator(), &Validator::strong_for(first.body()));
        let csp = first
            .headers()
            .iter()
            .find(|(name, _)| *name == "content-security-policy")
            .map(|(_, v)| v.to_owned());
        assert_eq!(csp.as_deref(), Some("script-src 'nonce-abcDEF123-_'"));
        assert!(
            first
                .headers()
                .iter()
                .any(|(name, value)| name == "content-type" && value.starts_with("text/html"))
        );
    }

    #[test]
    fn outcome_count_and_kinds_must_match_the_declared_slots() {
        let entry = entry();
        let nonce = Some("n0nce".to_owned());
        let failed = |input| {
            assemble(&entry, input, 1 << 20)
                .map(|_| ())
                .map_err(|e| e.kind())
        };
        assert_eq!(
            failed(AssemblyInput {
                outcomes: vec![SlotOutcome::Omitted],
                nonce: nonce.clone()
            }),
            Err(RenderCacheErrorKind::AssemblyFailed)
        );
        assert_eq!(
            failed(AssemblyInput {
                outcomes: vec![SlotOutcome::Omitted, SlotOutcome::Omitted],
                nonce: nonce.clone()
            }),
            Err(RenderCacheErrorKind::AssemblyFailed),
            "slot a is fail-document, not omit"
        );
        assert_eq!(
            failed(AssemblyInput {
                outcomes: vec![
                    SlotOutcome::Rendered(island("a", "doc-a", "A")),
                    SlotOutcome::Fallback
                ],
                nonce: nonce.clone()
            }),
            Err(RenderCacheErrorKind::AssemblyFailed),
            "slot b declares omit, not fallback"
        );
        assert_eq!(
            failed(AssemblyInput {
                outcomes: vec![
                    SlotOutcome::Rendered(island("b", "doc-a", "A")),
                    SlotOutcome::Omitted
                ],
                nonce: nonce.clone()
            }),
            Err(RenderCacheErrorKind::AssemblyFailed),
            "rendered slot name must match"
        );
        assert_eq!(
            failed(AssemblyInput {
                outcomes: vec![
                    SlotOutcome::Rendered(island("a", "doc-seed", "A")),
                    SlotOutcome::Omitted
                ],
                nonce: nonce.clone()
            }),
            Err(RenderCacheErrorKind::AssemblyFailed),
            "a rendered island may not reuse a shell island's document key"
        );
    }

    #[test]
    fn the_nonce_is_required_exactly_when_the_graph_needs_one() {
        let entry = entry();
        let outcomes = || {
            vec![
                SlotOutcome::Rendered(island("a", "doc-a", "A")),
                SlotOutcome::Omitted,
            ]
        };
        assert!(
            assemble(
                &entry,
                AssemblyInput {
                    outcomes: outcomes(),
                    nonce: None
                },
                1 << 20
            )
            .is_err()
        );
        assert!(
            assemble(
                &entry,
                AssemblyInput {
                    outcomes: outcomes(),
                    nonce: Some("bad nonce".to_owned())
                },
                1 << 20
            )
            .is_err()
        );
        let keys = keys();
        let (mut graph, shell) = graph_and_shell();
        graph
            .segments
            .retain(|segment| !matches!(segment, Segment::Nonce));
        graph.nonce_headers.clear();
        for index in 0..graph.slots.len() {
            graph.slots[index].surrounding =
                surrounding_digest(&graph, &shell, index).expect("surrounding");
        }
        let no_nonce = CompositeEntry::new(header(&keys), graph, shell).expect("entry");
        assert!(
            assemble(
                &no_nonce,
                AssemblyInput {
                    outcomes: outcomes(),
                    nonce: Some("n0nce".to_owned())
                },
                1 << 20
            )
            .is_err()
        );
        let document = assemble(
            &no_nonce,
            AssemblyInput {
                outcomes: outcomes(),
                nonce: None,
            },
            1 << 20,
        )
        .expect("assembles");
        let csp = document
            .headers()
            .iter()
            .find(|(name, _)| *name == "content-security-policy")
            .map(|(_, v)| v.to_owned());
        assert_eq!(
            csp.as_deref(),
            Some("script-src 'nonce-OLDNONCE'"),
            "an untemplated header replays as stored"
        );
    }

    #[test]
    fn fallback_inserts_the_declared_fragment() {
        let keys = keys();
        let (mut graph, shell) = graph_and_shell();
        graph.slots[1].on_failure = SlotFailurePolicy::Fallback {
            html: "<p>unavailable</p>".to_owned(),
        };
        let entry = CompositeEntry::new(header(&keys), graph, shell).expect("entry");
        let document = assemble(
            &entry,
            AssemblyInput {
                outcomes: vec![
                    SlotOutcome::Rendered(island("a", "doc-a", "A")),
                    SlotOutcome::Fallback,
                ],
                nonce: Some("n0nce".to_owned()),
            },
            1 << 20,
        )
        .expect("assembles");
        assert!(
            std::str::from_utf8(document.body())
                .expect("utf8")
                .contains("<p>mid</p><p>unavailable</p></body>")
        );
    }

    #[test]
    fn the_assembled_body_is_bounded() {
        let entry = entry();
        let input = AssemblyInput {
            outcomes: vec![
                SlotOutcome::Rendered(island("a", "doc-a", &"x".repeat(200))),
                SlotOutcome::Omitted,
            ],
            nonce: Some("n0nce".to_owned()),
        };
        assert_eq!(
            assemble(&entry, input, 128)
                .map(|_| ())
                .map_err(|e| e.kind()),
            Err(RenderCacheErrorKind::AssemblyFailed)
        );
    }
}
