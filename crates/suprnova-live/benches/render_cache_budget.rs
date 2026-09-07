//! Complete L0 hot hit (`C64`) and Composite assembly (`C64+4`) under
//! architecture performance budget v1, measured in an isolated,
//! single-threaded process with a benchmark-only counting global allocator.
//! On-demand: never a gate step. See docs/implementation/benchmarking.md.
#![allow(
    unsafe_code,
    reason = "benchmark-only counting global allocator required by the Complete L0 budget row"
)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::BTreeMap;
use std::error::Error;
use std::hint::black_box;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use std::{fs, iter};

use bytes::Bytes;
use http::header::{AGE, CACHE_CONTROL, CONTENT_TYPE, ETAG, REFERRER_POLICY, VARY};
use http::{Method, Response, StatusCode};
use serde::Serialize;
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::identity::{
    BuildId, ContentDigest, IslandSlot, KeyId, RouteIdentity, UnixMillis,
};
use suprnova_live::mount::DocumentMountKey;
use suprnova_live::render_cache::coherence::{FreshnessState, evaluate_freshness};
use suprnova_live::render_cache::composite::{
    AssemblyInput, CheckedIsland, CompositeEntry, HeaderPiece, HeaderTemplate, Segment,
    SegmentGraph, ShellIsland, SlotFailurePolicy, SlotOutcome, StitchSlot, assemble, fresh_nonce,
    surrounding_digest,
};
use suprnova_live::render_cache::entry::{
    CompleteEntry, EntryHeader, EntryLimits, SafeHeaders, decode, encode,
};
use suprnova_live::render_cache::generation::GenerationSet;
use suprnova_live::render_cache::hot::{HotEntry, HotRequest, serve_hot};
use suprnova_live::render_cache::key::{RenderKey, RenderKeyInput};
use suprnova_live::render_cache::policy::{
    FreshnessPolicy, RepresentationClass, SharedCachePolicy,
};
use suprnova_live::render_cache::store::{
    MemoryRenderStore, MemoryStoreLimits, PublicationFence, PublishOutcome,
};
use suprnova_live::render_cache::variance::VarianceDescriptor;
use suprnova_live::view::{TrustedHtml, TrustedMarkupReason};
use suprnova_live_test_support::bench_environment::{self, EnvironmentEvidence, percentile};

// -------------------------------------------------------------------------
// Counting global allocator
// -------------------------------------------------------------------------

/// Forwards every request to the system allocator unchanged and, while
/// armed, counts the call and the bytes it asked for. Only a benchmark ever
/// installs this; the library itself forbids `unsafe`.
struct Counting;

/// Whether the measured window is open. Set and cleared on the bench's only
/// thread, which the isolation guard proves is the process's only thread.
static ARMED: AtomicBool = AtomicBool::new(false);
/// Allocation calls counted since the window opened.
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
/// Bytes requested since the window opened.
static BYTES: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every method forwards its arguments to `System` unchanged and
// returns exactly what `System` returned, so this allocator's contract is
// `System`'s contract. Counting reads and writes only atomics and never
// allocates, so it cannot re-enter the allocator.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        // SAFETY: forwarded unchanged to the system allocator.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from `System.alloc` with this `layout`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        // SAFETY: forwarded unchanged to the system allocator.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count(new_size);
        // SAFETY: `ptr` came from `System.alloc` with this `layout`, and
        // `new_size` is forwarded unchanged.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

/// Records one allocation while the window is open. Deallocation is never
/// counted: the budget is a bound on allocations a request performs, not on
/// the memory it eventually returns.
fn count(size: usize) {
    if ARMED.load(Ordering::Relaxed) {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(size, Ordering::Relaxed);
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// An open measurement window. Held for exactly the work being measured;
/// [`Armed::stop`] closes it and reports the totals.
struct Armed;

impl Armed {
    /// Zeroes the counters and opens the window.
    fn start() -> Self {
        ALLOCATIONS.store(0, Ordering::Relaxed);
        BYTES.store(0, Ordering::Relaxed);
        ARMED.store(true, Ordering::SeqCst);
        Self
    }

    /// Closes the window and returns `(allocations, bytes)`.
    fn stop(self) -> (usize, usize) {
        ARMED.store(false, Ordering::SeqCst);
        (
            ALLOCATIONS.load(Ordering::Relaxed),
            BYTES.load(Ordering::Relaxed),
        )
    }
}

// -------------------------------------------------------------------------
// Budget constants
// -------------------------------------------------------------------------

/// Iterations run before anything is measured, per workload.
const WARMUP_ITERATIONS: usize = 200;
/// Timing samples taken per workload.
const MEASURED_SAMPLES: usize = 40;
/// Iterations inside one timing sample; the sample is the batch divided by
/// this, so one clock read covers work far larger than the clock's own cost.
const BATCH_ITERATIONS: usize = 50;
/// Armed single requests in the allocation pass.
const ALLOCATION_PASSES: usize = 100;
/// The `C64` body, in bytes.
const C64_BODY_BYTES: usize = 65_536;
/// Dependency generations the `C64` entry observed at render.
const C64_DEPENDENCIES: usize = 12;
/// One `C64+4` island's rendered bytes.
const SLOT_BYTES: usize = 4_096;
/// Identity-bound stitch slots in the `C64+4` shell.
const SLOTS: usize = 4;
/// Release-profile latency ceiling for the `C64` hot hit.
const C64_P95_CAP_MICROSECONDS: f64 = 250.0;
/// Release-profile latency ceiling for `C64+4` assembly.
const C64_PLUS_4_P95_CAP_MICROSECONDS: f64 = 2_000.0;
/// Heap allocations one `C64` measured request may perform.
const ALLOCATIONS_CAP: usize = 4;
/// Allocated bytes per byte of source content `C64+4` assembly may spend.
const COPY_RATIO_CAP: f64 = 2.0;

/// Fresh interval of the `C64` entry, in milliseconds.
const C64_FRESH_MS: u64 = 300_000;
/// Stale-servable interval of the `C64` entry, in milliseconds.
const C64_STALE_SERVABLE_MS: u64 = 60_000;
/// Stale-on-error interval of the `C64` entry, in milliseconds.
const C64_STALE_ON_ERROR_MS: u64 = 300_000;
/// Publication instant every fixture uses.
const PUBLISHED_AT_MS: u64 = 1_700_000_000_000;
/// Stored content type of both fixtures.
const CONTENT_TYPE_VALUE: &str = "text/html; charset=utf-8";
/// The one extra replayable header the `C64` entry stores.
///
/// The architecture note names `x-frame-options`; that name is outside
/// [`SafeHeaders`]'s replayable allowlist, so the nearest allowlisted
/// security header stands in and the workload still measures one stored
/// header replayed beside the formed ones.
const REFERRER_POLICY_VALUE: &str = "same-origin";
/// The `Cache-Control` a `C64` hit must serve, pinned literally rather than
/// recomputed through the same formatter the response used.
const C64_CACHE_CONTROL: &str = "private, max-age=300";
/// Milliseconds a seeded `C64` body's promotion deadline still has at the
/// measured instant. Its `Cache-Control` shrinks with the clock, so that
/// one value cannot be precomputed and the hit forms it per request.
const C64_SEED_REMAINING_MS: u64 = 45_000;
/// The `Cache-Control` the seeded variant must serve, pinned literally.
const C64_SEEDED_CACHE_CONTROL: &str = "private, max-age=45";
/// The assembly bound `C64+4` runs under.
const MAX_ASSEMBLED_BYTES: usize = 8 << 20;
/// Literal shell pieces around the nonce hole and the four slots.
const SHELL_LITERALS: usize = SLOTS + 2;

// -------------------------------------------------------------------------
// Fixtures
// -------------------------------------------------------------------------

/// The signing ring both fixtures are stored under. Fixed bytes, so a run
/// is reproducible; nothing signed here leaves the process.
fn key_ring() -> Result<SnapshotKeyRing, Box<dyn Error>> {
    let active = KeyRecord::new(
        KeyId::parse("render-cache-budget")?,
        RootKey::new(vec![23_u8; 32])?,
        UnixMillis::new(0),
        UnixMillis::new(u64::MAX / 2),
        UnixMillis::new(u64::MAX),
    )?;
    Ok(SnapshotKeyRing::new(active, Vec::new())?)
}

/// The fence every publication in this bench carries.
const fn fence() -> PublicationFence {
    PublicationFence {
        epoch: 1,
        generation_digest: [0_u8; 32],
        token: 1,
    }
}

/// One representation identity: two route parameters, one declared query
/// parameter, a trusted host, `text/html`, no negotiated encoding, build
/// `bench`, epoch 1, and no declared variance.
fn key_input(pattern: &str, route_seed: u8) -> Result<RenderKeyInput, Box<dyn Error>> {
    let mut params = BTreeMap::new();
    params.insert("category".to_owned(), "outerwear".to_owned());
    params.insert("page".to_owned(), "3".to_owned());
    let mut query = BTreeMap::new();
    query.insert("sort".to_owned(), "price-asc".to_owned());
    Ok(RenderKeyInput {
        route: RouteIdentity::from_bytes(&[route_seed; 32])?,
        route_pattern: pattern.to_owned(),
        params,
        query,
        host: Some("shop.example".to_owned()),
        media: "text/html".to_owned(),
        encoding: None,
        build: BuildId::parse("bench")?,
        epoch: 1,
        variance: VarianceDescriptor::new(),
    })
}

/// Exactly [`C64_BODY_BYTES`] of deterministic document-shaped bytes.
fn c64_body() -> Result<Bytes, Box<dyn Error>> {
    let head = b"<!doctype html><html><head><title>catalog</title></head><body><main>";
    let tail = b"</main></body></html>";
    let padding = C64_BODY_BYTES
        .checked_sub(head.len() + tail.len())
        .ok_or_else(|| io::Error::other("the C64 body frame is larger than the body"))?;
    let mut body = Vec::with_capacity(C64_BODY_BYTES);
    body.extend_from_slice(head);
    body.extend(iter::repeat_n(b'x', padding));
    body.extend_from_slice(tail);
    if body.len() != C64_BODY_BYTES {
        return Err(io::Error::other("the C64 body is not exactly 64 KiB").into());
    }
    Ok(Bytes::from(body))
}

/// A published Complete entry with everything one hot hit reads.
struct C64Fixture {
    keys: SnapshotKeyRing,
    input: RenderKeyInput,
    freshness: FreshnessPolicy,
    store: MemoryRenderStore,
    method: Method,
    body: Bytes,
    etag: String,
    cache_control: &'static str,
}

impl C64Fixture {
    /// The plain workload entry: no public seed, so every header value is
    /// precomputed at publication.
    fn fresh() -> Result<Self, Box<dyn Error>> {
        Self::new("/catalog/{category}", 7, None, C64_CACHE_CONTROL)
    }

    /// The same entry with a public seed deadline still ahead of the
    /// measured instant, so its `Cache-Control` is formed per request.
    fn seeded() -> Result<Self, Box<dyn Error>> {
        Self::new(
            "/catalog/{category}/seeded",
            9,
            Some(PUBLISHED_AT_MS + C64_SEED_REMAINING_MS),
            C64_SEEDED_CACHE_CONTROL,
        )
    }

    /// Builds the entry, stores it exactly as the framework stores one
    /// (encode then decode), prepares it for hot service, and publishes it.
    fn new(
        pattern: &str,
        route_seed: u8,
        seed_deadline_ms: Option<u64>,
        cache_control: &'static str,
    ) -> Result<Self, Box<dyn Error>> {
        let keys = key_ring()?;
        let freshness =
            FreshnessPolicy::new(C64_FRESH_MS, C64_STALE_SERVABLE_MS, C64_STALE_ON_ERROR_MS)?;
        let input = key_input(pattern, route_seed)?;
        let key = RenderKey::derive(&input, &keys)?;
        let mut observed = GenerationSet::default();
        for index in 0..C64_DEPENDENCIES {
            observed.insert_digest([u8::try_from(index)?; 32], 1)?;
        }
        let header = EntryHeader {
            key: key.clone(),
            class: RepresentationClass::PublicShared,
            variance: VarianceDescriptor::new(),
            published_at_ms: PUBLISHED_AT_MS,
            fresh_ms: C64_FRESH_MS,
            stale_servable_ms: C64_STALE_SERVABLE_MS,
            stale_on_error_ms: C64_STALE_ON_ERROR_MS,
            observed,
            epoch: 1,
            seed_deadline_ms,
            status: 200,
            headers: SafeHeaders::from_pairs([
                ("content-type", CONTENT_TYPE_VALUE),
                ("referrer-policy", REFERRER_POLICY_VALUE),
            ])?,
            content_encoding: None,
        };
        let encoded = encode(&CompleteEntry::new(header, c64_body()?), &keys)?;
        let stored = decode(&encoded, &keys, &EntryLimits::default())?
            .into_complete()
            .ok_or_else(|| io::Error::other("the stored C64 entry decoded as Composite"))?;
        let etag = stored.validator().etag();
        let hot = Arc::new(HotEntry::prepare(
            stored,
            SharedCachePolicy::Private,
            &freshness,
            PUBLISHED_AT_MS,
            fence(),
        )?);
        let body = hot.entry().body().clone();
        let store = MemoryRenderStore::new(MemoryStoreLimits {
            max_entries: 16,
            max_bytes: 16 << 20,
        });
        let outcome = store.publish_hot(&key, encoded, hot, fence(), PUBLISHED_AT_MS);
        if outcome != PublishOutcome::Published {
            return Err(
                io::Error::other(format!("the C64 fixture did not publish: {outcome:?}")).into(),
            );
        }
        Ok(Self {
            keys,
            input,
            freshness,
            store,
            method: Method::GET,
            body,
            etag,
            cache_control,
        })
    }

    /// The measured request: request facts through key derivation, L0
    /// retrieval, freshness evaluation, conditional evaluation, and response
    /// formation to a finished `http::Response<Bytes>`. Host conversion into
    /// a framework response type is outside it.
    fn measured_request(&self, if_none_match: Option<&str>, now_ms: u64) -> Response<Bytes> {
        let key = RenderKey::derive(&self.input, &self.keys).expect("bounded fixture derives");
        let hot = self.store.hot_get(&key).expect("the fixture is published");
        let header = hot.entry().header();
        let state = evaluate_freshness(
            &self.freshness,
            header.class,
            hot.published_at_ms(),
            now_ms,
            header.seed_deadline_ms,
        );
        assert_eq!(state, FreshnessState::Fresh, "the fixture is served fresh");
        serve_hot(
            &hot,
            HotRequest {
                method: &self.method,
                if_none_match,
                now_ms,
            },
            None,
        )
    }

    /// One timing sample: microseconds per request across a whole batch.
    fn timed_batch(&self, if_none_match: Option<&str>, now_ms: u64) -> f64 {
        let started = Instant::now();
        for _ in 0..BATCH_ITERATIONS {
            black_box(self.measured_request(if_none_match, now_ms));
        }
        elapsed_microseconds(started) / BATCH_ITERATIONS as f64
    }

    /// Everything the response must be, checked in every profile before any
    /// number is measured. A guard failure fails the bench.
    fn assert_correctness_guards(&self) -> Result<(), Box<dyn Error>> {
        let now_ms = PUBLISHED_AT_MS;
        let response = self.measured_request(None, now_ms);
        expect(response.status(), StatusCode::OK, "the hit status")?;
        expect(response.body().len(), C64_BODY_BYTES, "the served length")?;
        expect(
            header_text(&response, &ETAG),
            Some(self.etag.as_str()),
            "the entity tag",
        )?;
        expect(
            header_text(&response, &CACHE_CONTROL),
            Some(self.cache_control),
            "the cache directive",
        )?;
        expect(
            header_text(&response, &AGE),
            Some("0"),
            "the age at publication",
        )?;
        expect(
            header_text(&response, &CONTENT_TYPE),
            Some(CONTENT_TYPE_VALUE),
            "the content type",
        )?;
        expect(
            header_text(&response, &REFERRER_POLICY),
            Some(REFERRER_POLICY_VALUE),
            "the replayed stored header",
        )?;
        expect(
            header_text(&response, &VARY),
            None,
            "the vary of an entry that declares no variance",
        )?;
        if !self.body_is_shared(&response) {
            return Err(
                io::Error::other("the hit copied the stored body instead of sharing it").into(),
            );
        }

        let not_modified = self.measured_request(Some(&self.etag), now_ms);
        expect(
            not_modified.status(),
            StatusCode::NOT_MODIFIED,
            "the conditional status",
        )?;
        expect(not_modified.body().len(), 0, "the conditional body length")?;
        expect(
            header_text(&not_modified, &ETAG),
            Some(self.etag.as_str()),
            "the conditional entity tag",
        )?;
        let unrelated = self.measured_request(Some("\"sha256-unrelated\""), now_ms);
        expect(
            unrelated.status(),
            StatusCode::OK,
            "the status of an unrelated entity tag",
        )?;
        Ok(())
    }

    /// Whether the response body is the stored allocation itself rather
    /// than a copy of it: same pointer, same length.
    fn body_is_shared(&self, response: &Response<Bytes>) -> bool {
        response.body().as_ptr() == self.body.as_ptr()
            && response.body().len() == self.body.len()
            && self.body.len() == C64_BODY_BYTES
    }
}

/// A Composite entry whose shell is [`C64_BODY_BYTES`] long, cut by one
/// nonce hole and four identity-bound stitch slots.
struct CompositeFixture {
    entry: CompositeEntry,
    nonce: String,
    assembled_bytes: usize,
    source_bytes: usize,
}

impl CompositeFixture {
    /// Builds the shell, the graph, each slot's surrounding digest, and the
    /// entry that binds them.
    fn new() -> Result<Self, Box<dyn Error>> {
        let keys = key_ring()?;
        let literals = shell_literals()?;
        let shell = Bytes::from(literals.concat());
        if shell.len() != C64_BODY_BYTES {
            return Err(io::Error::other("the C64+4 shell is not exactly 64 KiB").into());
        }
        let mut segments = Vec::with_capacity(2 * SHELL_LITERALS);
        segments.push(Segment::Literal {
            len: u32::try_from(literals[0].len())?,
        });
        segments.push(Segment::Nonce);
        for index in 0..SLOTS {
            segments.push(Segment::Literal {
                len: u32::try_from(literals[index + 1].len())?,
            });
            segments.push(Segment::Slot {
                index: u16::try_from(index)?,
            });
        }
        segments.push(Segment::Literal {
            len: u32::try_from(literals[SHELL_LITERALS - 1].len())?,
        });
        let mut graph = SegmentGraph {
            segments,
            slots: (0..SLOTS).map(stitch_slot).collect::<Result<_, _>>()?,
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
        for index in 0..SLOTS {
            graph.slots[index].surrounding = surrounding_digest(&graph, &shell, index)?;
        }
        let input = key_input("/catalog/{category}/stitched", 8)?;
        let header = EntryHeader {
            key: RenderKey::derive(&input, &keys)?,
            class: RepresentationClass::PublicShellStitched,
            variance: VarianceDescriptor::new(),
            published_at_ms: PUBLISHED_AT_MS,
            fresh_ms: C64_FRESH_MS,
            stale_servable_ms: C64_STALE_SERVABLE_MS,
            stale_on_error_ms: C64_STALE_ON_ERROR_MS,
            observed: GenerationSet::default(),
            epoch: 1,
            seed_deadline_ms: None,
            status: 200,
            headers: SafeHeaders::from_pairs([("content-type", CONTENT_TYPE_VALUE)])?,
            content_encoding: None,
        };
        let entry = CompositeEntry::new(header, graph, shell)?;
        let nonce = fresh_nonce()?;
        let source_bytes = C64_BODY_BYTES + SLOTS * SLOT_BYTES;
        Ok(Self {
            assembled_bytes: source_bytes + nonce.len(),
            source_bytes,
            entry,
            nonce,
        })
    }

    /// One request's outcomes: four rendered islands of exactly
    /// [`SLOT_BYTES`] each, plus this request's nonce. Built outside every
    /// measured window, so the numbers describe assembly and nothing else.
    fn assembly_input(&self) -> Result<AssemblyInput, Box<dyn Error>> {
        let mut outcomes = Vec::with_capacity(SLOTS);
        for index in 0..SLOTS {
            outcomes.push(SlotOutcome::Rendered(island(index)?));
        }
        Ok(AssemblyInput {
            outcomes,
            nonce: Some(self.nonce.clone()),
        })
    }

    /// Assembly must produce exactly the shell, the four islands, and the
    /// nonce, in that order, and must render the nonce into its templated
    /// header. Checked in every profile, before any number is measured.
    fn assert_correctness_guards(&self) -> Result<(), Box<dyn Error>> {
        let assembled = assemble(&self.entry, self.assembly_input()?, MAX_ASSEMBLED_BYTES)?;
        expect(
            assembled.body().len(),
            self.assembled_bytes,
            "the assembled length",
        )?;
        let body = std::str::from_utf8(assembled.body())?;
        expect(
            body.starts_with("<!doctype html>"),
            true,
            "the assembled document opening",
        )?;
        expect(
            body.matches(self.nonce.as_str()).count(),
            1,
            "the nonce holes the assembled body fills",
        )?;
        let mut cursor = 0;
        for index in 0..SLOTS {
            let marker = format!("data-suprnova-live-root=\"slot-{index}\"");
            expect(body.matches(&marker).count(), 1, "one island per slot")?;
            let at = body
                .find(&marker)
                .ok_or_else(|| io::Error::other("an island is missing from the assembled body"))?;
            expect(at > cursor, true, "the islands land in slot order")?;
            cursor = at;
        }
        let nonce_headers = assembled
            .headers()
            .iter()
            .filter(|(name, value)| {
                *name == "content-security-policy" && value.contains(self.nonce.as_str())
            })
            .count();
        expect(nonce_headers, 1, "the assembled nonce header")?;
        Ok(())
    }
}

/// The six literal shell pieces in segment order, summing to exactly
/// [`C64_BODY_BYTES`]. Each piece carries a distinct marker so the four
/// surrounding digests cover distinct windows.
fn shell_literals() -> Result<[Vec<u8>; SHELL_LITERALS], Box<dyn Error>> {
    let fixed: [&[u8]; SHELL_LITERALS] = [
        b"<!doctype html><html><head><meta charset=\"utf-8\"><script nonce=\"",
        b"\"></script></head><body><main><section id=\"a\">",
        b"</section><section id=\"b\">",
        b"</section><section id=\"c\">",
        b"</section><section id=\"d\">",
        b"</section></main></body></html>",
    ];
    let fixed_total: usize = fixed.iter().map(|piece| piece.len()).sum();
    let padding = C64_BODY_BYTES
        .checked_sub(fixed_total)
        .ok_or_else(|| io::Error::other("the C64+4 shell frame is larger than the shell"))?;
    let per_piece = padding / (SHELL_LITERALS - 1);
    let mut literals: [Vec<u8>; SHELL_LITERALS] =
        std::array::from_fn(|index| fixed[index].to_vec());
    let mut remaining = padding;
    for (index, literal) in literals.iter_mut().enumerate().skip(1) {
        let count = if index == SHELL_LITERALS - 1 {
            remaining
        } else {
            per_piece
        };
        literal.extend(iter::repeat_n(b'a' + u8::try_from(index)?, count));
        remaining -= count;
    }
    Ok(literals)
}

/// One identity-bound stitch slot. Its `surrounding` is filled in once the
/// whole graph exists, since the digest covers the shell around it.
fn stitch_slot(index: usize) -> Result<StitchSlot, Box<dyn Error>> {
    Ok(StitchSlot {
        route: RouteIdentity::from_bytes(&[9_u8; 32])?.to_base64url(),
        slot: format!("slot-{index}"),
        document_key: format!("doc-slot-{index}"),
        component: "app.catalog.panel".to_owned(),
        contract_digest: ContentDigest::from_bytes(&[2_u8; 32])?.to_base64url(),
        protocol: 1,
        build: "bench".to_owned(),
        parameters: "{}".to_owned(),
        flags: BTreeMap::new(),
        on_failure: SlotFailurePolicy::FailDocument,
        surrounding: String::new(),
    })
}

/// One rendered island of exactly [`SLOT_BYTES`], bound to slot `index`.
fn island(index: usize) -> Result<CheckedIsland, Box<dyn Error>> {
    let slot = format!("slot-{index}");
    let document_key = format!("doc-slot-{index}");
    let opening = format!(
        "<div data-suprnova-live-root=\"{slot}\" data-suprnova-live-document-key=\"{document_key}\">"
    );
    let closing = "</div>";
    let padding = SLOT_BYTES
        .checked_sub(opening.len() + closing.len())
        .ok_or_else(|| io::Error::other("the island frame is larger than one slot"))?;
    let mut html = String::with_capacity(SLOT_BYTES);
    html.push_str(&opening);
    html.extend(iter::repeat_n('y', padding));
    html.push_str(closing);
    if html.len() != SLOT_BYTES {
        return Err(io::Error::other("an island is not exactly one slot wide").into());
    }
    Ok(CheckedIsland::new(
        TrustedHtml::framework_generated(
            html,
            TrustedMarkupReason::new("render-cache budget island")?,
        )?,
        IslandSlot::parse(&slot)?,
        DocumentMountKey::parse(&document_key)?,
    ))
}

// -------------------------------------------------------------------------
// Measurement
// -------------------------------------------------------------------------

/// What a whole set of armed passes observed for one request shape.
#[derive(Default)]
struct AllocationLedger {
    passes: usize,
    max_allocations: usize,
    max_bytes: usize,
    distribution: BTreeMap<usize, usize>,
}

impl AllocationLedger {
    /// Records one armed pass.
    fn record(&mut self, allocations: usize, bytes: usize) {
        self.passes += 1;
        self.max_allocations = self.max_allocations.max(allocations);
        self.max_bytes = self.max_bytes.max(bytes);
        *self.distribution.entry(allocations).or_insert(0) += 1;
    }

    /// The distribution as `count=passes` pairs, for the one-line summary.
    fn distribution_text(&self) -> String {
        self.distribution
            .iter()
            .map(|(allocations, passes)| format!("{allocations}={passes}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Fails when any pass exceeded the cap, naming the count and the shape.
    fn assert_within_cap(&self, workload: &str) -> Result<(), Box<dyn Error>> {
        if self.max_allocations > ALLOCATIONS_CAP {
            return Err(io::Error::other(format!(
                "{workload} performed {} heap allocations in one measured request, above the cap of {ALLOCATIONS_CAP}; distribution {}",
                self.max_allocations,
                self.distribution_text()
            ))
            .into());
        }
        Ok(())
    }
}

/// Sorted per-request microseconds for one workload variant.
struct Timing {
    p50: f64,
    p95: f64,
    samples: usize,
}

impl Timing {
    /// Sorts the samples and takes the two nearest-rank percentiles.
    fn from_samples(mut samples: Vec<f64>) -> Self {
        samples.sort_by(f64::total_cmp);
        Self {
            p50: percentile(&samples, 0.50),
            p95: percentile(&samples, 0.95),
            samples: samples.len(),
        }
    }

    /// The zero record a debug profile reports, where timing never runs.
    const fn unmeasured() -> Self {
        Self {
            p50: 0.0,
            p95: 0.0,
            samples: 0,
        }
    }

    /// Fails when the ninety-fifth percentile reached `cap`.
    fn assert_within_cap(&self, workload: &str, cap: f64) -> Result<(), Box<dyn Error>> {
        if self.p95 > cap {
            return Err(io::Error::other(format!(
                "{workload} reached p95 {:.3} us, above the {cap:.0} us ceiling",
                self.p95
            ))
            .into());
        }
        Ok(())
    }
}

/// Microseconds since `started`.
fn elapsed_microseconds(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1_000_000.0
}

/// One header's text, or `None` when the response does not carry it.
fn header_text<'a>(response: &'a Response<Bytes>, name: &http::HeaderName) -> Option<&'a str> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
}

/// Fails with a message naming the fact, what it was, and what it had to be.
fn expect<T>(observed: T, required: T, fact: &str) -> Result<(), Box<dyn Error>>
where
    T: PartialEq + std::fmt::Debug,
{
    if observed == required {
        return Ok(());
    }
    Err(io::Error::other(format!("{fact} is {observed:?}, not {required:?}")).into())
}

/// Refuses to measure anything unless this process is the isolated
/// single-threaded Linux process the allocation counts assume. A second
/// thread could allocate inside an armed window and the count would be a
/// number about the process, not about the request.
fn assert_isolated() -> Result<(), Box<dyn Error>> {
    if !cfg!(target_os = "linux") {
        return Err(io::Error::other(
            "the render-cache budget harness measures allocations in an isolated Linux process; the S1 reference environment is Linux and this platform is not supported",
        )
        .into());
    }
    let status = fs::read_to_string("/proc/self/status")?;
    let threads = status
        .lines()
        .find_map(|line| line.strip_prefix("Threads:"))
        .map(str::trim)
        .ok_or_else(|| io::Error::other("/proc/self/status does not report a thread count"))?;
    if threads != "1" {
        return Err(io::Error::other(format!(
            "the allocation measurement needs an isolated single-threaded process; /proc/self/status reports {threads} threads"
        ))
        .into());
    }
    Ok(())
}

// -------------------------------------------------------------------------
// Result record
// -------------------------------------------------------------------------

/// One run of both workloads, as written to the result file.
#[derive(Serialize)]
struct BudgetResult {
    schema_version: u8,
    profile: &'static str,
    measured_at_unix_ms: u128,
    environment: EnvironmentEvidence,
    c64: C64Result,
    c64_plus_4: CompositeResult,
}

/// The Complete L0 hot hit.
#[derive(Serialize)]
struct C64Result {
    body_bytes: usize,
    dependencies: usize,
    warmup_iterations: usize,
    measured_samples: usize,
    batch_iterations: usize,
    p50_microseconds: f64,
    p95_microseconds: f64,
    p95_cap_microseconds: f64,
    allocation_passes: usize,
    allocations_max: usize,
    allocations_cap: usize,
    allocations_distribution: BTreeMap<usize, usize>,
    allocated_bytes_max: usize,
    body_shared: bool,
    not_modified: NotModifiedResult,
    seed_deadline: SeedDeadlineResult,
}

/// The same hit answered 304 by a matching entity tag.
#[derive(Serialize)]
struct NotModifiedResult {
    p50_microseconds: f64,
    p95_microseconds: f64,
    allocation_passes: usize,
    allocations_max: usize,
    allocations_distribution: BTreeMap<usize, usize>,
    allocated_bytes_max: usize,
}

/// The same hit for a body that embeds a public seed deadline, whose
/// `Cache-Control` shrinks with the clock and so is the one header value a
/// hit cannot precompute. Reported so the entry shape with the most
/// per-request work is measured rather than reasoned about.
#[derive(Serialize)]
struct SeedDeadlineResult {
    seed_remaining_ms: u64,
    allocation_passes: usize,
    allocations_max: usize,
    allocations_cap: usize,
    allocations_distribution: BTreeMap<usize, usize>,
    allocated_bytes_max: usize,
}

/// Composite assembly of a 64 KiB shell with four 4 KiB islands.
#[derive(Serialize)]
struct CompositeResult {
    shell_bytes: usize,
    slots: usize,
    slot_bytes: usize,
    assembled_bytes: usize,
    warmup_iterations: usize,
    measured_samples: usize,
    batch_iterations: usize,
    p50_microseconds: f64,
    p95_microseconds: f64,
    p95_cap_microseconds: f64,
    allocation_passes: usize,
    allocated_bytes_max: usize,
    copy_ratio_max: f64,
    copy_ratio_cap: f64,
}

/// Where the result is written: `SUPRNOVA_LIVE_BENCH_RESULT` when set,
/// otherwise the checked-in file beside this crate's manifest.
fn result_path() -> PathBuf {
    std::env::var_os("SUPRNOVA_LIVE_BENCH_RESULT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("benchmarks/render-cache-budget-v1.json")
        })
}

/// Writes through a temporary file and one rename, so a reader never sees
/// a half-written result.
fn write_result(result: &BudgetResult, path: &Path) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    let mut bytes = serde_json::to_vec_pretty(result)?;
    bytes.push(b'\n');
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, path)?;
    Ok(())
}

// -------------------------------------------------------------------------
// Workloads
// -------------------------------------------------------------------------

/// Warmup, the allocation pass, and the release-only timing pass for the
/// Complete L0 hot hit, its conditional 304 variant, and the seed-deadline
/// entry shape whose `Cache-Control` cannot be precomputed. Only the first
/// two are timed; the third is measured for allocations alone, since it is
/// the shape that decides whether the cap has any headroom left.
fn run_c64(timed: bool) -> Result<C64Result, Box<dyn Error>> {
    let fixture = C64Fixture::fresh()?;
    let seeded = C64Fixture::seeded()?;
    fixture.assert_correctness_guards()?;
    seeded.assert_correctness_guards()?;
    let now_ms = PUBLISHED_AT_MS;

    for _ in 0..WARMUP_ITERATIONS {
        black_box(fixture.measured_request(None, now_ms));
        black_box(fixture.measured_request(Some(&fixture.etag), now_ms));
        black_box(seeded.measured_request(None, now_ms));
    }

    let mut full = AllocationLedger::default();
    let mut conditional = AllocationLedger::default();
    let mut seed_deadline = AllocationLedger::default();
    let mut body_shared = true;
    for _ in 0..ALLOCATION_PASSES {
        let armed = Armed::start();
        let response = fixture.measured_request(None, now_ms);
        let (allocations, bytes) = armed.stop();
        body_shared &= fixture.body_is_shared(&response);
        drop(black_box(response));
        full.record(allocations, bytes);

        let armed = Armed::start();
        let response = fixture.measured_request(Some(&fixture.etag), now_ms);
        let (allocations, bytes) = armed.stop();
        drop(black_box(response));
        conditional.record(allocations, bytes);

        let armed = Armed::start();
        let response = seeded.measured_request(None, now_ms);
        let (allocations, bytes) = armed.stop();
        body_shared &= seeded.body_is_shared(&response);
        drop(black_box(response));
        seed_deadline.record(allocations, bytes);
    }
    if !body_shared {
        return Err(io::Error::other(
            "a measured C64 hit copied the stored body instead of sharing it",
        )
        .into());
    }
    full.assert_within_cap("the C64 hot hit")?;
    conditional.assert_within_cap("the C64 conditional hit")?;
    seed_deadline.assert_within_cap("the C64 seed-deadline hit")?;

    let (timing, conditional_timing) = if timed {
        let mut samples = Vec::with_capacity(MEASURED_SAMPLES);
        let mut conditional_samples = Vec::with_capacity(MEASURED_SAMPLES);
        for _ in 0..MEASURED_SAMPLES {
            samples.push(fixture.timed_batch(None, now_ms));
            conditional_samples.push(fixture.timed_batch(Some(&fixture.etag), now_ms));
        }
        (
            Timing::from_samples(samples),
            Timing::from_samples(conditional_samples),
        )
    } else {
        (Timing::unmeasured(), Timing::unmeasured())
    };

    println!(
        "C64 Complete hot hit: allocations max={} ({}) bytes={} p50={:.3}us p95={:.3}us body_shared={body_shared}",
        full.max_allocations,
        full.distribution_text(),
        full.max_bytes,
        timing.p50,
        timing.p95
    );
    println!(
        "C64 conditional 304: allocations max={} ({}) bytes={} p50={:.3}us p95={:.3}us",
        conditional.max_allocations,
        conditional.distribution_text(),
        conditional.max_bytes,
        conditional_timing.p50,
        conditional_timing.p95
    );
    println!(
        "C64 seed-deadline hit: allocations max={} ({}) bytes={}",
        seed_deadline.max_allocations,
        seed_deadline.distribution_text(),
        seed_deadline.max_bytes
    );
    if timed {
        timing.assert_within_cap("the C64 hot hit", C64_P95_CAP_MICROSECONDS)?;
        conditional_timing
            .assert_within_cap("the C64 conditional hit", C64_P95_CAP_MICROSECONDS)?;
    }

    Ok(C64Result {
        body_bytes: C64_BODY_BYTES,
        dependencies: C64_DEPENDENCIES,
        warmup_iterations: WARMUP_ITERATIONS,
        measured_samples: timing.samples,
        batch_iterations: BATCH_ITERATIONS,
        p50_microseconds: timing.p50,
        p95_microseconds: timing.p95,
        p95_cap_microseconds: C64_P95_CAP_MICROSECONDS,
        allocation_passes: full.passes,
        allocations_max: full.max_allocations,
        allocations_cap: ALLOCATIONS_CAP,
        allocations_distribution: full.distribution,
        allocated_bytes_max: full.max_bytes,
        body_shared,
        not_modified: NotModifiedResult {
            p50_microseconds: conditional_timing.p50,
            p95_microseconds: conditional_timing.p95,
            allocation_passes: conditional.passes,
            allocations_max: conditional.max_allocations,
            allocations_distribution: conditional.distribution,
            allocated_bytes_max: conditional.max_bytes,
        },
        seed_deadline: SeedDeadlineResult {
            seed_remaining_ms: C64_SEED_REMAINING_MS,
            allocation_passes: seed_deadline.passes,
            allocations_max: seed_deadline.max_allocations,
            allocations_cap: ALLOCATIONS_CAP,
            allocations_distribution: seed_deadline.distribution,
            allocated_bytes_max: seed_deadline.max_bytes,
        },
    })
}

/// Warmup, the allocated-bytes pass, and the release-only timing pass for
/// Composite assembly. The request's outcomes are built outside every
/// measured window, so both numbers describe `assemble` alone.
fn run_c64_plus_4(timed: bool) -> Result<CompositeResult, Box<dyn Error>> {
    let fixture = CompositeFixture::new()?;
    fixture.assert_correctness_guards()?;

    for _ in 0..WARMUP_ITERATIONS {
        let input = fixture.assembly_input()?;
        black_box(assemble(&fixture.entry, input, MAX_ASSEMBLED_BYTES)?);
    }

    let mut ledger = AllocationLedger::default();
    for _ in 0..ALLOCATION_PASSES {
        let input = fixture.assembly_input()?;
        let armed = Armed::start();
        let assembled = assemble(&fixture.entry, input, MAX_ASSEMBLED_BYTES);
        let (allocations, bytes) = armed.stop();
        let assembled = assembled?;
        expect(
            assembled.body().len(),
            fixture.assembled_bytes,
            "a measured C64+4 assembled length",
        )?;
        drop(black_box(assembled));
        ledger.record(allocations, bytes);
    }
    let copy_ratio = ledger.max_bytes as f64 / fixture.source_bytes as f64;

    let timing = if timed {
        let mut samples = Vec::with_capacity(MEASURED_SAMPLES);
        for _ in 0..MEASURED_SAMPLES {
            let mut inputs = Vec::with_capacity(BATCH_ITERATIONS);
            for _ in 0..BATCH_ITERATIONS {
                inputs.push(fixture.assembly_input()?);
            }
            let started = Instant::now();
            for input in inputs {
                black_box(assemble(&fixture.entry, input, MAX_ASSEMBLED_BYTES)?);
            }
            samples.push(elapsed_microseconds(started) / BATCH_ITERATIONS as f64);
        }
        Timing::from_samples(samples)
    } else {
        Timing::unmeasured()
    };

    println!(
        "C64+4 Composite assembly: allocated bytes max={} over {} source bytes, copy ratio {copy_ratio:.4} p50={:.3}us p95={:.3}us",
        ledger.max_bytes, fixture.source_bytes, timing.p50, timing.p95
    );
    if copy_ratio > COPY_RATIO_CAP {
        return Err(io::Error::other(format!(
            "C64+4 assembly allocated {} bytes over {} source bytes, a copy ratio of {copy_ratio:.4} above the cap of {COPY_RATIO_CAP:.1}",
            ledger.max_bytes, fixture.source_bytes
        ))
        .into());
    }
    if timed {
        timing.assert_within_cap("C64+4 assembly", C64_PLUS_4_P95_CAP_MICROSECONDS)?;
    }

    Ok(CompositeResult {
        shell_bytes: C64_BODY_BYTES,
        slots: SLOTS,
        slot_bytes: SLOT_BYTES,
        assembled_bytes: fixture.assembled_bytes,
        warmup_iterations: WARMUP_ITERATIONS,
        measured_samples: timing.samples,
        batch_iterations: BATCH_ITERATIONS,
        p50_microseconds: timing.p50,
        p95_microseconds: timing.p95,
        p95_cap_microseconds: C64_PLUS_4_P95_CAP_MICROSECONDS,
        allocation_passes: ledger.passes,
        allocated_bytes_max: ledger.max_bytes,
        copy_ratio_max: copy_ratio,
        copy_ratio_cap: COPY_RATIO_CAP,
    })
}

fn main() {
    if let Err(error) = run() {
        eprintln!("render cache budget failed: {error}");
        std::process::exit(1);
    }
}

/// Guards, both workloads, and the result file. A debug build runs the
/// correctness and allocation checks and skips timing, as `snapshot_budget`
/// does; only a release run writes a result.
fn run() -> Result<(), Box<dyn Error>> {
    assert_isolated()?;
    let timed = !cfg!(debug_assertions);

    let c64 = run_c64(timed)?;
    let c64_plus_4 = run_c64_plus_4(timed)?;

    if !timed {
        println!(
            "render cache budget debug contract and allocation checks only; release timing skipped"
        );
        return Ok(());
    }

    let mut environment = bench_environment::collect();
    environment.database = "not_used_by_render_cache_budget_benchmark";
    environment.provider_versions = BTreeMap::from([
        ("render_store", "in_process_memory_l0_v1"),
        ("snapshot_key_ring", "in_process_v1"),
    ]);
    let result = BudgetResult {
        schema_version: 1,
        profile: "release",
        measured_at_unix_ms: SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
        environment,
        c64,
        c64_plus_4,
    };
    let path = result_path();
    write_result(&result, &path)?;
    println!(
        "render cache budget written to {} environment={}",
        path.display(),
        result.environment.classification
    );
    Ok(())
}
