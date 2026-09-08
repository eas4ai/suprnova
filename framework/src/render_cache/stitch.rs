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
//! An assembled document is never a repeat of an earlier one - it carries a
//! nonce and island identities minted for this request alone - so a
//! Composite hit never answers 304, and a Composite entry with any slot in
//! it is sent `private, no-store` - on the leader's own render of it as much
//! as on every later assembly - so no shared browser profile can replay one
//! principal's islands to the next visitor (see
//! [`cache_control_override_for`]).
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
use suprnova_live::mount::{DocumentMountKey, DocumentMountScope, PrivateMountRequest};
use suprnova_live::render_cache::composite::{
    AssembledDocument, AssemblyInput, CheckedIsland, CompositeEntry, HeaderPiece, HeaderTemplate,
    MAX_NONCE_HEADERS, MAX_NONCE_HOLES, MAX_STITCH_SLOTS, ParsedSlot, Segment, SegmentGraph,
    SlotFailurePolicy, SlotOutcome, StitchSlot, assemble, fresh_nonce, surrounding_digest,
};
use suprnova_live::render_cache::entry::{CompleteEntry, DecodedEntry, EntryHeader};
use suprnova_live::render_cache::hot::{HotRequest, ResponseParts, respond as respond_with_engine};
use suprnova_live::snapshot::MountedDocumentPath;

use crate::http::{HttpResponse, Request, Response};
use crate::live::{LiveMountKind, LiveRuntime, StitchSlotDescriptor};
use crate::middleware::Next;
use crate::telemetry::metrics::Metrics;

use super::middleware::{FoundEntry, LookupOutcome, complete_response, hot_response};
use super::telemetry as render_cache_telemetry;
use super::{RenderCache, RenderCachePolicy, collector, live::LiveDocumentFacts};

/// A hit the RenderCache middleware decoded, checked, and handed to the
/// route chain instead of serving itself.
///
/// Everything [`serve_prepared`] needs to answer the request once the chain
/// has run. The freshness decision and the `now_ms` it was taken at are both
/// fixed at lookup time, before the chain ran: the entry is served under the
/// state the middleware actually decided on, and the chain's own duration is
/// not added to the `Age` the client sees.
pub(crate) struct PreparedHit {
    /// The stored representation the lookup found: a Complete L0 entry
    /// prepared for hot service, or one this request decoded.
    pub(crate) entry: FoundEntry,
    /// The route's effective policy, for the served cache metadata.
    pub(crate) policy: RenderCachePolicy,
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
    let published_at_ms = hit.entry.published_at_ms();
    match hit.entry {
        // A hot entry is a Complete one whose header values were formed at
        // publication: it is served through the engine's hot builder,
        // exactly as a non-stitched hit is, once the chain has let it
        // through. The runtime is read back here rather than carried on the
        // hit, the same way `assemble_hit` reads it: a runtime replaced
        // between the lookup and here is one this response can no longer be
        // attributed to, so the route renders instead.
        FoundEntry::Hot(hot) => match RenderCache::runtime() {
            Some(runtime) => Ok(hot_response(
                &runtime,
                &hot,
                request.method(),
                request.header("if-none-match"),
                hit.now_ms,
                hit.warning,
            )),
            None => next(request).await,
        },
        FoundEntry::Decoded(decoded) => match decoded.entry {
            DecodedEntry::Complete(entry) => match complete_response(
                request.method(),
                request.header("if-none-match"),
                &hit.policy,
                &entry,
                published_at_ms,
                hit.now_ms,
                hit.warning,
            ) {
                Some(response) => Ok(response),
                // A stored entry that cannot be formed into a valid response
                // is a store defect; the route renders it instead of this
                // serving something malformed. It is the same outcome the
                // non-stitched twin records for the same defect
                // (`middleware::deliver_hit` counts a `Miss` when
                // `hit_response` gives it nothing), and `fail_document`
                // additionally counts it under this module's own
                // assembly-outcome label, so the one store-defect path a
                // stitched route can take is not the only one with no
                // counter behind it.
                None => {
                    LookupOutcome::Miss.record();
                    fail_document(request, next).await
                }
            },
            DecodedEntry::Composite(entry) => {
                assemble_hit(
                    request,
                    next,
                    entry,
                    hit.policy,
                    published_at_ms,
                    hit.now_ms,
                    hit.warning,
                )
                .await
            }
        },
    }
}

/// Serves a Composite entry by assembling it for *this* request.
///
/// The shell is shared; the islands in it are not. Every slot the graph
/// declares is re-derived from the live mount catalog, checked against the
/// identities the entry stored, re-authorized through the same request
/// context a handler's own mount would build, and mounted again here and
/// now. Nothing about an island is replayed from storage: the entry carries
/// only the island's *declaration*, never its markup or its snapshot, so an
/// assembled document can only ever contain islands this request's own
/// principal was authorized for. Reauthorization is per request, never
/// cached.
///
/// Failure is per slot and follows the slot's own declared policy (see
/// [`render_slot`] for what counts as a slot failure): `Omit` and
/// `Fallback` are recorded as such and assembly continues, `FailDocument`
/// abandons assembly for the whole document. Anything that is not a slot's
/// business - a runtime that is not installed, a path or a stored
/// declaration that will not parse, a shell whose islands cannot be
/// reserved, an exhausted randomness source, or an engine assembly that
/// rejects the result - fails the document as a whole rather than serving a
/// partial one. Every one of those paths ends at [`fail_document`], which
/// counts the outcome and lets the route's own handler answer, uncached,
/// exactly as it would on a miss.
///
/// `now_ms` is the instant the cache middleware evaluated freshness at,
/// before the chain ran, so `Age` measures the entry's age and not the time
/// the chain and these mounts took.
async fn assemble_hit(
    request: Request,
    next: Next,
    entry: CompositeEntry,
    policy: RenderCachePolicy,
    published_at_ms: u64,
    now_ms: u64,
    warning: Option<&'static str>,
) -> Response {
    let Some(runtime) = RenderCache::runtime() else {
        return fail_document(request, next).await;
    };
    let Ok(live) = LiveRuntime::bind() else {
        return fail_document(request, next).await;
    };
    let Ok(path) = MountedDocumentPath::parse(request.path()) else {
        return fail_document(request, next).await;
    };
    let graph = entry.graph();
    // The public-seed islands that stayed inside the shell already own their
    // document keys, so the scope has to know about them before a single
    // slot is mounted: without this a stitched slot could re-mount under a
    // key the assembled document already contains and the browser would see
    // two islands claiming one identity.
    let mut scope = DocumentMountScope::new();
    for island in &graph.shell_islands {
        let Ok(key) = DocumentMountKey::parse(&island.document_key) else {
            return fail_document(request, next).await;
        };
        if scope.reserve_existing(key).is_err() {
            return fail_document(request, next).await;
        }
    }
    let mut outcomes = Vec::with_capacity(graph.slots.len());
    for slot in &graph.slots {
        let Ok(parsed) = slot.parse() else {
            return fail_document(request, next).await;
        };
        match render_slot(&request, &live, &mut scope, &parsed, &path).await {
            Ok(island) => {
                count_slot("rendered");
                outcomes.push(SlotOutcome::Rendered(island));
            }
            Err(()) => match &parsed.on_failure {
                SlotFailurePolicy::Omit => {
                    count_slot("omitted");
                    outcomes.push(SlotOutcome::Omitted);
                }
                SlotFailurePolicy::Fallback { .. } => {
                    count_slot("fallback");
                    outcomes.push(SlotOutcome::Fallback);
                }
                SlotFailurePolicy::FailDocument => {
                    count_slot("failed");
                    return fail_document(request, next).await;
                }
            },
        }
    }
    // A graph needs a nonce when the shell has a hole where one was, or when
    // a stored header's value carried one; either way it is minted here, per
    // request, so no two visitors are ever sent the same one.
    let nonce = if entry.needs_nonce() {
        match fresh_nonce() {
            Ok(nonce) => Some(nonce),
            Err(_) => return fail_document(request, next).await,
        }
    } else {
        None
    };
    let Ok(document) = assemble(
        &entry,
        AssemblyInput { outcomes, nonce },
        runtime.limits.max_body_bytes,
    ) else {
        return fail_document(request, next).await;
    };
    let Some(response) = respond(
        &request,
        &policy,
        &entry,
        &document,
        published_at_ms,
        now_ms,
        warning,
    ) else {
        // Unreachable for an assembled document (see `respond`); the route
        // renders it rather than this serving something malformed, and the
        // attempt is counted as the abandoned assembly it is.
        return fail_document(request, next).await;
    };
    count_assembly("assembled");
    Ok(response)
}

/// Re-mounts one declared slot under this request's own authority.
///
/// `Err(())` means the slot could not be resolved *for this request*, and
/// the caller applies the slot's declared failure policy to it. There is
/// deliberately only one error: an unregistered mount, a declaration that
/// no longer matches the catalog, a refused request context (an anonymous
/// or logged-out visitor included), and a rejected mount are all the same
/// fact from the assembler's point of view - this island is not available
/// to this principal - and the declaration, not this function, decides what
/// that means for the document.
///
/// The identity comparison is the whole point of storing typed identities
/// in the slot rather than a component name alone. A registration is
/// accepted only when it is still identity-bound (a mount redeclared as a
/// public seed belongs in the shell, not in a slot) and its component,
/// contract digest, protocol, document key, and build all match what was
/// stored. Anything else is drift between the stored entry and the running
/// build, and drift is a slot failure, never a substitution.
///
/// The mount runs inside [`collector::slot_scope`] for the same reason
/// `LiveDocument::mount` does: whatever it reads belongs to this island,
/// which is re-rendered on every hit, and must never be attributed to the
/// shared shell. There is no collector open on the hit path today, so the
/// wrapper is inert here; it is kept so the two mount sites cannot drift
/// apart if one ever is opened.
async fn render_slot(
    request: &Request,
    live: &LiveRuntime,
    scope: &mut DocumentMountScope,
    slot: &ParsedSlot,
    path: &MountedDocumentPath,
) -> Result<CheckedIsland, ()> {
    let registration = live
        .stitch_registration(&slot.route, &slot.slot)
        .ok_or(())?;
    if registration.kind != LiveMountKind::IdentityBound
        || registration.selection.component() != &slot.component
        || registration.selection.contract_digest() != &slot.contract_digest
        || registration.selection.protocol() != slot.protocol
        || registration.document_key != slot.document_key
        || registration.build != slot.build
    {
        return Err(());
    }
    let context = live
        .validate_request_context(request, registration.selection.clone())
        .map_err(|_| ())?;
    let output = collector::slot_scope(
        live.mount_private_component(
            scope,
            PrivateMountRequest::new(
                slot.document_key.clone(),
                slot.parameters.clone(),
                slot.flags.clone(),
            )
            .with_document_path(path.clone()),
            &context,
        ),
    )
    .await
    .map_err(|_| ())?;
    // The slot comes from the mount's own metadata, not from the stored
    // declaration, so the assembler's identity check compares the island
    // that was actually produced against the slot it is about to be dropped
    // into rather than comparing a stored value with itself. The document
    // key has no such counterpart - neither `MountMetadata` nor
    // `PrivateMountOutput` reports one - so the declaration's key stands,
    // and it is the key the mount was requested under two lines above.
    let (html, metadata) = output.into_document_parts();
    Ok(CheckedIsland::new(
        html,
        metadata.slot().clone(),
        slot.document_key.clone(),
    ))
}

/// Abandons assembly and lets the route's own handler answer, uncached.
///
/// This is the only outcome besides a fully assembled document: there is no
/// partial response. The handler renders the document itself, exactly as it
/// does on a miss, and the stored entry is left untouched.
async fn fail_document(request: Request, next: Next) -> Response {
    count_assembly("fail_document");
    next(request).await
}

/// Builds the served response for an assembled document.
///
/// The counterpart of
/// [`complete_response`](super::middleware::complete_response) for a
/// representation that had to be assembled first, and formed through the
/// same engine builder as every other RenderCache response
/// ([`suprnova_live::render_cache::hot::respond`]). Three things differ, and
/// all three follow from the bytes being new.
///
/// The replayable headers come from the assembled document rather than from
/// the stored header, because a nonce-bearing header (a
/// `Content-Security-Policy`, typically) has been rebuilt around the nonce
/// minted for this request; replaying the stored value would declare the
/// leader's nonce over a body carrying somebody else's.
///
/// **A Composite response never answers 304.** `If-None-Match` is not
/// evaluated here at all: every assembly is a distinct representation - a
/// fresh nonce, fresh instance identities - so a 304 would tell the client
/// to pair the body it already has with the nonce headers minted for *this*
/// request, and those describe a document it has never seen. The validator
/// is still emitted, still strong over exactly the bytes sent, and still
/// honest about which representation this is; it simply never matches on a
/// later request, which is the truth. `HEAD` still sends the headers with
/// no body.
///
/// **A slotted entry is `private, no-store`.** The bytes contain islands
/// mounted for one principal under authority re-derived for one request. A
/// `max-age` on that would let a shared browser profile replay one
/// principal's islands to whoever sits down next, and would skip the
/// per-request reauthorization for the whole window. A zero-slot Composite
/// has no per-principal bytes in it - only a per-request nonce - so it
/// keeps the class's private `max-age` from
/// [`cache_control_value`] as any other private representation would. The
/// decision itself is [`cache_control_override_for`], which the leader's
/// own render asks as well, so the directive does not depend on which code
/// path produced the bytes.
///
/// Everything else is the shared contract: `Vary` from the declared
/// variance, `Age` from the publication instant, and `Warning` when the
/// entry was served stale.
/// The `Cache-Control` a stitched route's response is pinned to, or `None`
/// when the class's computed value stands.
///
/// A Composite entry with at least one slot is `private, no-store`. The
/// bytes it describes hold islands mounted for one principal under
/// authority re-derived for one request; a `max-age` on them would let a
/// shared browser profile replay one principal's islands to whoever sits
/// down next, and would skip the per-request reauthorization for the whole
/// window. A zero-slot Composite has no per-principal bytes in it - only a
/// per-request nonce - so it keeps the class's private `max-age` like any
/// other private representation.
///
/// The rule is about what the bytes contain, not about which code path
/// produced them, so both writers of a stitched route's response ask this
/// one function: [`respond`] for every later assembly, and
/// `middleware::finish_fresh_render` for the leader's own rendered
/// document, whose islands are that leader's and are no more storable than
/// an assembly of the same shell.
pub(crate) fn cache_control_override_for(entry: &CompositeEntry) -> Option<&'static str> {
    if entry.graph().slots.is_empty() {
        None
    } else {
        Some("private, no-store")
    }
}

fn respond(
    request: &Request,
    policy: &RenderCachePolicy,
    entry: &CompositeEntry,
    document: &AssembledDocument,
    published_at_ms: u64,
    now_ms: u64,
    warning: Option<&'static str>,
) -> Option<HttpResponse> {
    let header = entry.header();
    let freshness = policy.freshness();
    // A zero-slot Composite has no per-principal bytes in it - only a
    // per-request nonce - so it keeps the class's private `max-age` the
    // engine computes for any other private representation; a slotted one
    // is pinned to `private, no-store`, for the reason this function's doc
    // gives. The leader's own render of the same entry asks the same
    // function, so the two answers cannot drift.
    let cache_control_override = cache_control_override_for(entry);
    let formed = respond_with_engine(
        ResponseParts {
            status: header.status,
            class: header.class,
            shared: policy.shared(),
            freshness: &freshness,
            // The assembled document's own headers, not the stored ones: a
            // nonce-bearing header has been rebuilt around the nonce minted
            // for this request.
            headers: document.headers(),
            variance: &header.variance,
            validator: document.validator(),
            body: document.body(),
            published_at_ms,
            seed_deadline_ms: header.seed_deadline_ms,
            cache_control_override,
        },
        // `None` rather than the request's own value: an assembly never
        // answers 304, for the reason this function's doc gives, and the
        // engine only ever forms one from an `If-None-Match` it was handed.
        HotRequest {
            method: request.method(),
            if_none_match: None,
            now_ms,
        },
        warning,
    );
    match formed {
        Ok(response) => Some(HttpResponse::from_engine_response(response)),
        // Unreachable for an assembled document - every header in it came
        // through `SafeHeaders`, which bounds and checks each pair - so this
        // is the same fail-closed treatment a defective stored entry gets in
        // [`complete_response`](super::middleware::complete_response): the
        // caller abandons the assembly and the route renders.
        Err(error) => {
            tracing::warn!(
                target: "suprnova::render_cache",
                kind = %error,
                "an assembled document could not be formed into a response; \
                 it was not served",
            );
            None
        }
    }
}

/// Counts one composite assembly attempt under its closed outcome label.
fn count_assembly(outcome: &'static str) {
    Metrics::counter(render_cache_telemetry::STITCH_ASSEMBLIES)
        .inc_with(&[(render_cache_telemetry::OUTCOME, outcome)]);
}

/// Counts one slot outcome inside a composite assembly.
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
