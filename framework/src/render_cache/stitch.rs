//! Publishing and serving stitched routes: [`build_composite_entry`] cuts a
//! shared shell out of one render's document on a miss, and
//! [`serve_prepared`] answers the hit afterwards - a Complete entry replays
//! after the route chain ran; a Composite entry is assembled from
//! re-mounted islands.
//!
//! [`RepresentationClass::PublicShellStitched`](super::RepresentationClass::PublicShellStitched)
//! is the one class whose gate has to run again on every hit, so the global
//! [`RenderCacheMiddleware`](super::middleware::RenderCacheMiddleware) never
//! answers such a route where it stands: it attaches the decoded entry to
//! the request (see `Request::attach_prepared_hit`) and calls the next
//! layer, so the route's own authorization guard, tenant middleware, and
//! anything else the route declared all run exactly as they do on a miss.
//! The Live completion middleware - the last middleware before the handler -
//! is what finally serves the hit, through `serve_prepared`. A request the
//! chain refuses first never reaches here at all, and the prepared hit is
//! dropped unread with the request.
//!
//! The consequence, stated once: `PublicShellStitched` is meaningful only on
//! routes whose chain ends in the Live completion middleware; elsewhere every
//! hit is discarded and the route renders as if nothing were cached.
//!
//! A route under this class must not rewrite the response body in route
//! middleware after the Live document rendered it. Such middleware runs again
//! on every hit, so its output would be baked into the stored representation
//! on the miss and then applied a second time on the hit, with the entry's
//! `ETag` describing bytes no client ever received. The design closes this at
//! publication rather than here: `LiveDocument::render` records a SHA-256 of
//! the body it produced, and the composite publisher declines any stitched
//! document whose response body digest differs from that recording -
//! zero-island documents included - so no stitched entry ever holds
//! post-processed bytes. A route that does rewrite its body is simply never
//! published and is served uncached on every request.

use bytes::Bytes;
use sha2::{Digest as _, Sha256};
use suprnova_live::render_cache::composite::{
    CompositeEntry, HeaderPiece, HeaderTemplate, MAX_NONCE_HEADERS, MAX_NONCE_HOLES,
    MAX_STITCH_SLOTS, Segment, SegmentGraph, StitchSlot, surrounding_digest,
};
use suprnova_live::render_cache::entry::{CompleteEntry, DecodedEntry, EntryHeader};

use crate::http::{Request, Response};
use crate::live::StitchSlotDescriptor;
use crate::middleware::Next;
use crate::telemetry::metrics::Metrics;

use super::RenderCachePolicy;
use super::collector;
use super::live::LiveDocumentFacts;
use super::middleware::conditional_response;
use super::telemetry as render_cache_telemetry;

/// A hit the RenderCache middleware decoded, checked, and handed to the
/// route chain instead of serving itself.
///
/// Everything [`serve_prepared`] needs to answer the request once the chain
/// has run. The freshness decision and the `now_ms` it was taken at are both
/// fixed at lookup time, before the chain ran: the entry is served under the
/// state the middleware actually decided on, and the chain's own duration is
/// not added to the `Age` the client sees.
pub(crate) struct PreparedHit {
    /// The decoded stored representation.
    pub(crate) entry: DecodedEntry,
    /// The route's effective policy, for the served cache metadata.
    pub(crate) policy: RenderCachePolicy,
    /// When the entry was published, for `Age`.
    pub(crate) published_at_ms: u64,
    /// The instant the middleware evaluated freshness at, fixed before the
    /// chain ran; `Age` is measured from it, not from when the hit is served.
    pub(crate) now_ms: u64,
    /// The `Warning` header a stale-servable entry carries, if any.
    pub(crate) warning: Option<&'static str>,
}

/// Marks the handler boundary and serves the hit the RenderCache middleware
/// prepared, if there is one.
///
/// [`collector::begin_handler`] runs first on every request that gets this
/// far, hit or miss: this is the last middleware before the route handler,
/// so everything after it is a content read, and a stitched route is
/// classified from content reads alone. Without it a stitched route's
/// content bucket would be empty on every request and nothing would ever be
/// published (see `CollectorReport::handler_began`).
pub(crate) async fn serve_prepared(mut request: Request, next: Next) -> Response {
    collector::begin_handler();
    let Some(hit) = request.take_prepared_hit() else {
        return next(request).await;
    };
    let hit = *hit;
    match hit.entry {
        DecodedEntry::Complete(entry) => conditional_response(
            request.method().as_str(),
            request.header("if-none-match"),
            &hit.policy,
            &entry,
            hit.published_at_ms,
            hit.now_ms,
            hit.warning,
        ),
        DecodedEntry::Composite(entry) => {
            assemble_hit(
                request,
                next,
                entry,
                hit.policy,
                hit.published_at_ms,
                hit.now_ms,
                hit.warning,
            )
            .await
        }
    }
}

/// Serves a Composite entry.
///
/// Task 8 replaces this body with request-time assembly. Until then a
/// Composite entry is served the way a fail-document outcome is: by the
/// route's own handler, uncached.
async fn assemble_hit(
    request: Request,
    next: Next,
    _entry: CompositeEntry,
    _policy: RenderCachePolicy,
    _published_at_ms: u64,
    _now_ms: u64,
    _warning: Option<&'static str>,
) -> Response {
    count_assembly("fail_document");
    next(request).await
}

/// Counts one composite assembly attempt under its closed outcome label.
fn count_assembly(outcome: &'static str) {
    Metrics::counter(render_cache_telemetry::STITCH_ASSEMBLIES)
        .inc_with(&[(render_cache_telemetry::OUTCOME, outcome)]);
}

/// Counts one slot outcome inside a composite assembly.
#[allow(
    dead_code,
    reason = "Task 8's assembler is the only producer of slot outcomes"
)]
fn count_slot(outcome: &'static str) {
    Metrics::counter(render_cache_telemetry::STITCH_SLOTS)
        .inc_with(&[(render_cache_telemetry::OUTCOME, outcome)]);
}

/// Builds the entry a stitched route's render publishes as, or `None` to
/// decline publication entirely.
///
/// The return is a [`DecodedEntry`] rather than a [`CompositeEntry`]
/// because the Complete-versus-Composite decision belongs here, *after*
/// every validity check below, not before it: a stitched document that
/// mounted no identity-bound island is still a stitched document, and the
/// checks that make a stitched shell safe to share (a trustworthy capture,
/// and a response body that is exactly what the document rendered) have to
/// hold for it too. A document that fails any of them is declined; only a
/// document that passes them all is then published as a Composite entry
/// when it has islands to cut out, or as a Complete shell when it does not.
///
/// The checks, in order:
///
/// 1. The capture is a faithful account of the request: not marked
///    `invalid`, and holding exactly one captured slot per identity-bound
///    island the request mounted. A capture that lost an island would
///    otherwise publish that island's bytes inside shared shell bytes.
/// 2. The response body is byte-for-byte the body `LiveDocument::render`
///    produced. Route middleware that rewrites the body afterwards runs
///    again on every hit (a stitched hit goes through the whole chain), so
///    storing its output would apply it twice and leave the entry's
///    validator describing bytes no client ever received. This is also
///    what bounds everything below: the body is now provably the rendered
///    document, which the view renderer already bounded at the
///    configuration's `max_response_bytes`.
/// 3. Every captured island's bytes occur exactly once in that body, and
///    the occurrences do not overlap. An island that cannot be located
///    exactly once cannot be cut out, and a shell that kept it would be
///    that principal's markup and signed snapshot, shared.
///
/// Nothing here is fallible in the error sense: every rejection is a
/// decline, and the caller records it under the existing declined outcome.
pub(crate) fn build_composite_entry(
    header: EntryHeader,
    body: &[u8],
    facts: &LiveDocumentFacts,
) -> Option<DecodedEntry> {
    let capture = &facts.stitch;
    if capture.invalid
        || capture.slots.len() != facts.identity_bound_islands
        || capture.slots.len() > MAX_STITCH_SLOTS
    {
        return None;
    }
    let digest: [u8; 32] = Sha256::digest(body).into();
    if capture.document_digest != Some(digest) {
        return None;
    }
    // 1. Locate every island exactly once, then prove the placements are
    //    disjoint and put them in document order.
    let mut placed: Vec<(usize, usize, usize)> = Vec::with_capacity(capture.slots.len());
    for (index, slot) in capture.slots.iter().enumerate() {
        if slot.html.is_empty() {
            return None;
        }
        let mut found = find_all(body, &slot.html);
        if found.len() != 1 {
            return None;
        }
        let start = found.remove(0);
        placed.push((start, start + slot.html.len(), index));
    }
    placed.sort_by_key(|(start, _, _)| *start);
    if placed.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return None;
    }
    // Every check above has now run. A document with no identity-bound
    // island has nothing to cut out and is a finished shared answer.
    if capture.slots.is_empty() {
        return Some(DecodedEntry::Complete(CompleteEntry::new(
            header,
            Bytes::copy_from_slice(body),
        )));
    }
    // 2. Nonce holes: occurrences of this document's bootstrap nonce that
    //    lie outside every island. One inside an island is that island's
    //    own business - it is re-rendered on every hit and brings its own
    //    nonce with it - and is deliberately not a hole.
    let nonce = capture.nonce.as_deref().filter(|nonce| !nonce.is_empty());
    let mut holes: Vec<(usize, usize)> = Vec::new();
    if let Some(nonce) = nonce {
        for start in find_all(body, nonce.as_bytes()) {
            let end = start + nonce.len();
            if placed.iter().any(|(s, e, _)| start < *e && end > *s) {
                continue;
            }
            holes.push((start, end));
            if holes.len() > MAX_NONCE_HOLES {
                return None;
            }
        }
    }
    // 3. Walk the body once, cutting at every island and every hole: what
    //    is not cut out becomes the shell, and the cuts become typed
    //    segments. The shell therefore holds no island bytes at all.
    let mut cuts: Vec<(usize, usize, Cut)> = placed
        .iter()
        .map(|(start, end, index)| (*start, *end, Cut::Slot(*index)))
        .chain(holes.iter().map(|(start, end)| (*start, *end, Cut::Nonce)))
        .collect();
    cuts.sort_by_key(|(start, _, _)| *start);
    let mut segments = Vec::new();
    let mut shell = Vec::with_capacity(body.len());
    let mut slots = Vec::new();
    let mut cursor = 0usize;
    let mut slot_index: u16 = 0;
    for (start, end, cut) in cuts {
        if start > cursor {
            shell.extend_from_slice(&body[cursor..start]);
            segments.push(Segment::Literal {
                len: u32::try_from(start - cursor).ok()?,
            });
        }
        match cut {
            Cut::Slot(capture_index) => {
                segments.push(Segment::Slot { index: slot_index });
                slots.push(stitch_slot(&capture.slots[capture_index].descriptor));
                slot_index = slot_index.checked_add(1)?;
            }
            Cut::Nonce => segments.push(Segment::Nonce),
        }
        cursor = end;
    }
    if cursor < body.len() {
        shell.extend_from_slice(&body[cursor..]);
        segments.push(Segment::Literal {
            len: u32::try_from(body.len() - cursor).ok()?,
        });
    }
    // 4. Every replayable header whose value carries the nonce becomes a
    //    template, so a hit's fresh nonce reaches the header as well as the
    //    body and no stored header is left holding the miss's nonce as if
    //    it still described the response (the assembler rebuilds these
    //    names from the templates, overriding what was stored). Header-only
    //    templates are legitimate on their own: a document may declare the
    //    nonce in a header without the shell containing a hole.
    let mut nonce_headers = Vec::new();
    if let Some(nonce) = nonce {
        for (name, value) in header.headers.iter() {
            if !value.contains(nonce) {
                continue;
            }
            if nonce_headers.len() >= MAX_NONCE_HEADERS {
                return None;
            }
            let mut pieces = Vec::new();
            let mut rest = value;
            while let Some(at) = rest.find(nonce) {
                if at > 0 {
                    pieces.push(HeaderPiece::Text {
                        text: rest[..at].to_owned(),
                    });
                }
                pieces.push(HeaderPiece::Nonce);
                rest = &rest[at + nonce.len()..];
                if pieces.len() > MAX_NONCE_HOLES {
                    return None;
                }
            }
            if !rest.is_empty() {
                pieces.push(HeaderPiece::Text {
                    text: rest.to_owned(),
                });
            }
            nonce_headers.push(HeaderTemplate {
                name: name.to_owned(),
                pieces,
            });
        }
    }
    let shell = Bytes::from(shell);
    let mut graph = SegmentGraph {
        segments,
        slots,
        shell_islands: capture.shell_islands.clone(),
        nonce_headers,
    };
    // Filled from the graph and the shell alone, exactly as the assembler
    // recomputes it at hit time; a shell that drifted from the digest it
    // was recorded with is what that recomputation catches.
    for index in 0..graph.slots.len() {
        graph.slots[index].surrounding = surrounding_digest(&graph, &shell, index).ok()?;
    }
    CompositeEntry::new(header, graph, shell)
        .ok()
        .map(DecodedEntry::Composite)
}

/// One cut in the rendered body: an island to re-render on every hit, or a
/// nonce to regenerate on every hit.
enum Cut {
    /// The island captured at this index of `StitchCapture::slots`.
    Slot(usize),
    /// One occurrence of the document's bootstrap nonce.
    Nonce,
}

/// Every non-overlapping occurrence of `needle` in `haystack`, left to
/// right, as start offsets.
///
/// An empty needle never matches, so the scan always advances. The result
/// is bounded by `haystack.len() / needle.len()`, and every caller here
/// scans a body the digest check has already proven to be the rendered
/// document, which the view renderer bounded at the configuration's
/// `max_response_bytes`.
fn find_all(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    let mut found = Vec::new();
    if needle.is_empty() || needle.len() > haystack.len() {
        return found;
    }
    let mut at = 0usize;
    while at + needle.len() <= haystack.len() {
        if haystack[at..].starts_with(needle) {
            found.push(at);
            at += needle.len();
        } else {
            at += 1;
        }
    }
    found
}

/// The stored form of one captured island's declaration: the framework's
/// typed identities spelled as the bounded strings a stored entry carries.
///
/// `surrounding` is left empty here and filled once the graph and the shell
/// both exist, since the digest is over the shell bytes adjacent to the
/// slot rather than over anything the declaration knows.
fn stitch_slot(descriptor: &StitchSlotDescriptor) -> StitchSlot {
    StitchSlot {
        route: descriptor.route.to_base64url(),
        slot: descriptor.slot.as_str().to_owned(),
        document_key: descriptor.document_key.as_str().to_owned(),
        component: descriptor.component.as_str().to_owned(),
        contract_digest: descriptor.contract_digest.to_base64url(),
        protocol: descriptor.protocol,
        build: descriptor.build.as_str().to_owned(),
        parameters: descriptor.parameters.clone(),
        flags: descriptor
            .flags
            .iter()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect(),
        on_failure: descriptor.on_failure.clone(),
        surrounding: String::new(),
    }
}
