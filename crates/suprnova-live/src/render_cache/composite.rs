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

use super::entry::{EntryHeader, REPLAYABLE_HEADERS};
use super::{RenderCacheError, RenderCacheErrorKind};
use crate::canonical::CanonicalValue;
use crate::identity::{BuildId, ComponentName, ContentDigest, IslandSlot, RouteIdentity};
use crate::mount::{DocumentMountKey, MountFlags};

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
        let limits = crate::limits::InputLimits::new(MAX_SLOT_PARAMETER_BYTES, 32, 512, 4_096)
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
                || !template
                    .pieces
                    .iter()
                    .any(|piece| matches!(piece, HeaderPiece::Nonce))
            {
                return Err(invalid());
            }
            let text_len: usize = template
                .pieces
                .iter()
                .map(|piece| match piece {
                    HeaderPiece::Text { text } => text.len(),
                    HeaderPiece::Nonce => 0,
                })
                .sum();
            if text_len > 4_096
                || template.pieces.iter().any(|piece| match piece {
                    HeaderPiece::Text { text } => {
                        text.bytes().any(|b| matches!(b, b'\r' | b'\n' | 0))
                    }
                    HeaderPiece::Nonce => false,
                })
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

/// SHA-256 (base64url) over the shell bytes adjacent to slot `index`: the
/// last [`SURROUNDING_WINDOW_BYTES`] of the literal segment immediately
/// before the slot (empty when the previous segment is not a literal), a
/// zero byte, and the first [`SURROUNDING_WINDOW_BYTES`] of the literal
/// segment immediately after it (empty likewise). Computed from the graph
/// and the shell alone, so the assembler can recompute it without the
/// original document.
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
    hasher.update(before);
    hasher.update([0u8]);
    hasher.update(after);
    Ok(URL_SAFE_NO_PAD.encode(hasher.finalize()))
}

fn decode_digest(text: &str) -> Result<[u8; 32], RenderCacheError> {
    let bytes = URL_SAFE_NO_PAD.decode(text).map_err(|_| invalid())?;
    <[u8; 32]>::try_from(bytes).map_err(|_| invalid())
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
        let limits = crate::limits::InputLimits::new(max_header_bytes, 32, 512, 4_096)
            .map_err(|_| invalid())?;
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
    /// meaningfully observable.
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
    fn a_composite_entry_binds_header_graph_and_shell_into_a_structural_digest() {
        let keys = keys();
        let (graph, shell) = graph_and_shell();
        let entry =
            CompositeEntry::new(header(&keys), graph.clone(), shell.clone()).expect("entry");
        let same = CompositeEntry::new(header(&keys), graph.clone(), shell.clone()).expect("entry");
        assert_eq!(entry.structural_digest(), same.structural_digest());
        let mut other_shell = shell.to_vec();
        other_shell[0] ^= 1;
        let mut other_graph = graph.clone();
        other_graph.segments[0] = Segment::Literal {
            len: graph.segments[0].literal_len().expect("literal"),
        };
        let other = CompositeEntry::new(header(&keys), other_graph, Bytes::from(other_shell))
            .expect("entry");
        assert_ne!(entry.structural_digest(), other.structural_digest());
        assert_ne!(
            *entry.structural_digest(),
            <[u8; 32]>::from(sha2::Sha256::digest(&shell)),
            "the structural digest is not a body digest"
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
        assert!(!valid_nonce(&"a".repeat(257)));
    }
}
