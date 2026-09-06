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
/// document that passes them all is then published, and only then is its
/// form decided.
///
/// The steps, numbered the same here and in the body:
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
/// 4. Occurrences of the document's bootstrap nonce outside every island
///    are collected as holes (see [`collect_holes`]). A scan that reached
///    its own match limit declines outright, before any filtering: a
///    truncated scan cannot prove there is no further occurrence, and one
///    it did not see would be copied into the shared shell as a fixed
///    nonce while every hit rebuilt the header with a fresh one.
/// 5. Every replayable stored header whose value carries that nonce is
///    collected as a template.
/// 6. Only now is the form decided, and it can be decided because steps 3
///    to 5 between them enumerated everything that has to come out: every
///    island, every nonce occurrence, every nonce-bearing header. A
///    document with none of the three is a finished shared answer and is
///    published Complete. Anything else is published Composite - including
///    a document with *no* islands but a nonce, whose graph is a legal
///    zero-slot one: a Complete entry there would freeze the first
///    visitor's nonce into the stored body and the stored
///    `Content-Security-Policy` alike and replay both to everybody, which
///    is a nonce that proves nothing.
/// 7. The body is walked once, cutting at every island and every hole:
///    what is not cut out becomes the shell, the cuts become typed
///    segments, and each slot's surrounding digest is taken over the shell
///    that resulted. The shell therefore holds no island bytes and no
///    nonce occurrence at all - one inside an island leaves with the
///    island, and one outside every island is a hole, because step 4
///    declined rather than hand back a truncated list.
///
/// Nothing here is fallible in the error sense: every rejection is a
/// decline, and the caller records it under the existing declined outcome.
pub(crate) fn build_composite_entry(
    header: EntryHeader,
    body: &[u8],
    facts: &LiveDocumentFacts,
) -> Option<DecodedEntry> {
    // 1. The capture accounts for every identity-bound island, within bound.
    let capture = &facts.stitch;
    if capture.invalid
        || capture.slots.len() != facts.identity_bound_islands
        || capture.slots.len() > MAX_STITCH_SLOTS
    {
        return None;
    }
    // 2. The response body is the body the document rendered.
    let digest: [u8; 32] = Sha256::digest(body).into();
    if capture.document_digest != Some(digest) {
        return None;
    }
    // 3. Locate every island exactly once, then prove the placements are
    //    disjoint and put them in document order. `find_all` counts
    //    overlapping occurrences too, so an island whose bytes match at two
    //    positions is declined rather than half cut out; two is all the
    //    caller needs to know, since anything but exactly one declines.
    let mut placed: Vec<(usize, usize, usize)> = Vec::with_capacity(capture.slots.len());
    for (index, slot) in capture.slots.iter().enumerate() {
        if slot.html.is_empty() {
            return None;
        }
        let mut found = find_all(body, &slot.html, 2);
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
    // 4. Nonce holes, over the island ranges just proved disjoint. The
    //    helper declines for the whole document, so `?` carries that out.
    let nonce = capture.nonce.as_deref().filter(|nonce| !nonce.is_empty());
    let islands: Vec<(usize, usize)> = placed.iter().map(|(s, e, _)| (*s, *e)).collect();
    let holes = match nonce {
        Some(nonce) => collect_holes(body, nonce.as_bytes(), &islands)?,
        None => Vec::new(),
    };
    // 5. Every replayable header whose value carries the nonce becomes a
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
    // 6. Every check has now run and everything that has to be cut out is
    //    known. Only a document with nothing to cut out at all is a
    //    finished shared answer; see this function's own doc for why a
    //    nonce alone is enough to make it a Composite one.
    if capture.slots.is_empty() && holes.is_empty() && nonce_headers.is_empty() {
        return Some(DecodedEntry::Complete(CompleteEntry::new(
            header,
            Bytes::copy_from_slice(body),
        )));
    }
    // 7. Walk the body once: what is not cut out becomes the shell, and the
    //    cuts become typed segments. The shell therefore holds no island
    //    bytes, and no nonce occurrence either - one inside an island
    //    leaves with the island, and every occurrence outside every island
    //    is a hole, because `collect_holes` declined rather than hand back
    //    a list its scan had truncated.
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

/// The nonce holes to cut out of `body`: every occurrence of `nonce` that
/// lies outside every range in `islands`, as `(start, end)` pairs in
/// ascending order. `None` declines the whole document.
///
/// An occurrence inside an island is that island's own business - the
/// island is re-rendered on every hit and brings its own nonce with it - so
/// it is not a hole, and an occurrence overlapping a hole already taken is
/// skipped too, which keeps the holes disjoint from each other exactly as
/// they are from the islands.
///
/// The scan is bounded at [`MAX_NONCE_HOLES`] `+ 1` matches, and a scan that
/// reached that limit declines here, *before* any of that filtering. The
/// filtering only ever removes occurrences, so counting holes after it
/// would let a body with more occurrences than the bound pass whenever
/// enough of them happened to fall inside an island - and the ones the scan
/// never reached would then be copied into the shared shell verbatim,
/// leaving one visitor's nonce fixed in bytes every later visitor receives
/// while the assembled `Content-Security-Policy` carried a fresh one. A
/// truncated scan cannot prove completeness, so it is not asked to.
///
/// `islands` is bounded by [`MAX_STITCH_SLOTS`] and the returned vector by
/// the scan's own limit, so both are bounded before either is allocated.
fn collect_holes(
    body: &[u8],
    nonce: &[u8],
    islands: &[(usize, usize)],
) -> Option<Vec<(usize, usize)>> {
    let found = find_all(body, nonce, MAX_NONCE_HOLES + 1);
    if found.len() > MAX_NONCE_HOLES {
        return None;
    }
    let mut holes: Vec<(usize, usize)> = Vec::with_capacity(found.len());
    for start in found {
        let end = start + nonce.len();
        if islands.iter().any(|(s, e)| start < *e && end > *s)
            || holes.last().is_some_and(|(_, taken)| start < *taken)
        {
            continue;
        }
        holes.push((start, end));
    }
    // Unreachable: the scan above stops at `MAX_NONCE_HOLES + 1` matches and
    // declines at that count, and filtering only removes. Kept as the
    // invariant this function's whole contract with `SegmentGraph::validate`
    // rests on, rather than left to the reader to re-derive.
    if holes.len() > MAX_NONCE_HOLES {
        return None;
    }
    Some(holes)
}

/// Every occurrence of `needle` in `haystack`, left to right, as start
/// offsets, stopping once `limit` of them have been found.
///
/// Occurrences may overlap: the scan advances one byte past a match, not
/// one needle. A caller that needs disjoint results says so itself (see
/// step 4 of [`build_composite_entry`]); a caller that only needs to know
/// whether something occurs more than once gets the honest count rather
/// than a count that silently skipped a second, overlapping occurrence.
///
/// `limit` bounds the returned vector before it is allocated, so no caller
/// depends on the body's own size for its bound. An empty needle never
/// matches and a zero limit collects nothing, so the scan always
/// terminates. Each step skips ahead to the next byte equal to the
/// needle's first, which keeps the scan linear on any realistic body.
fn find_all(haystack: &[u8], needle: &[u8], limit: usize) -> Vec<usize> {
    if needle.is_empty() || needle.len() > haystack.len() || limit == 0 {
        return Vec::new();
    }
    let mut found = Vec::with_capacity(limit);
    let first = needle[0];
    // Never underflows: `needle.len() <= haystack.len()` was just checked.
    let last_start = haystack.len() - needle.len();
    let mut at = 0usize;
    while at <= last_start {
        let Some(offset) = haystack[at..=last_start]
            .iter()
            .position(|byte| *byte == first)
        else {
            break;
        };
        at += offset;
        if haystack[at..].starts_with(needle) {
            found.push(at);
            if found.len() >= limit {
                break;
            }
        }
        at += 1;
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

#[cfg(test)]
mod tests {
    use super::{MAX_NONCE_HOLES, collect_holes, find_all};

    /// `count` occurrences of the test nonce `XY`, each followed by a
    /// separator, so occurrence `n` starts at `3 * n` and no two overlap.
    fn body_with(count: usize) -> Vec<u8> {
        b"XY-".repeat(count)
    }

    /// Overlapping occurrences are counted, so a needle that matches twice
    /// is reported twice rather than once. This is what makes step 3 of
    /// [`super::build_composite_entry`] decline an island whose bytes could
    /// be cut in more than one place instead of cutting the first and
    /// leaving the second inside the shared shell.
    #[test]
    fn overlapping_occurrences_are_counted_separately() {
        assert_eq!(find_all(b"aaaa", b"aaa", 2), vec![0, 1]);
        assert_eq!(find_all(b"abab", b"abab", 2), vec![0]);
    }

    /// The limit stops the scan, so the returned vector is bounded before
    /// it is allocated rather than by the size of the body being scanned.
    #[test]
    fn the_limit_stops_the_scan() {
        assert_eq!(find_all(b"aaaa", b"a", 2), vec![0, 1]);
        assert_eq!(find_all(b"aaaa", b"a", 4), vec![0, 1, 2, 3]);
        assert!(find_all(b"aaaa", b"a", 0).is_empty());
    }

    /// Nothing occurs in nothing, and an empty needle never matches, so
    /// neither can make the scan run away or the caller cut at a
    /// zero-length range.
    #[test]
    fn an_empty_haystack_or_needle_finds_nothing() {
        assert!(find_all(b"", b"a", 2).is_empty());
        assert!(find_all(b"", b"", 2).is_empty());
        assert!(find_all(b"abc", b"", 2).is_empty());
        assert!(find_all(b"ab", b"abc", 2).is_empty());
    }

    /// Every occurrence in the shell becomes a hole, right up to the bound.
    #[test]
    fn every_shell_occurrence_within_the_bound_becomes_a_hole() {
        let body = body_with(MAX_NONCE_HOLES);
        let holes = collect_holes(&body, b"XY", &[]).expect("within bound");
        assert_eq!(holes.len(), MAX_NONCE_HOLES);
        assert_eq!(holes[0], (0, 2));
        assert_eq!(
            holes[MAX_NONCE_HOLES - 1],
            (3 * (MAX_NONCE_HOLES - 1), 3 * MAX_NONCE_HOLES - 1)
        );
    }

    /// One occurrence past the bound declines: 65 is what the scan's limit
    /// lets it see, and seeing the limit is what it declines on.
    #[test]
    fn one_occurrence_past_the_bound_declines() {
        let body = body_with(MAX_NONCE_HOLES + 1);
        assert!(collect_holes(&body, b"XY", &[]).is_none());
    }

    /// The regression this helper exists for: a truncated scan declines
    /// even when islands would have trimmed the *visible* holes back under
    /// the bound. With 66 occurrences of which two lie inside one island,
    /// filtering first left 63 holes and published a shell still carrying
    /// every occurrence the scan never reached.
    #[test]
    fn a_truncated_scan_declines_even_when_islands_would_trim_it_back_under_the_bound() {
        let body = body_with(MAX_NONCE_HOLES + 2);
        let island = (30usize, 36usize);
        assert_eq!(
            &body[island.0..island.1],
            b"XY-XY-",
            "the island covers two occurrences"
        );
        assert!(collect_holes(&body, b"XY", &[island]).is_none());
    }

    /// An occurrence straddling an island boundary belongs to the island,
    /// not to the shell, and dropping it is not by itself a reason to
    /// decline.
    #[test]
    fn an_occurrence_straddling_an_island_boundary_is_excluded_without_declining() {
        let body = body_with(3);
        let holes = collect_holes(&body, b"XY", &[(4, 6)]).expect("nothing exceeded a bound");
        assert_eq!(holes, vec![(0, 2), (6, 8)]);
    }
}
