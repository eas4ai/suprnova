//! Public-seed Live documents cache as Complete entries capped at the seed
//! deadline; identity-bound islands and no-store intents never store, even
//! when a handler mounts one and never calls `LiveDocument::render`.
use crate::live_dogfood_support;
use crate::render_cache_live_support;

use bytes::Bytes;
use live_dogfood_support::{DOCUMENT_PATH, DogfoodCounter, PRIVATE_DOCUMENT_PATH};
use render_cache_live_support::{
    CAPTURE_NONCE, CAPTURE_PATH, FAILED_MOUNT_FALLBACK, FAILED_MOUNT_PATH, RAW_PATH,
    SEAM_CONTROL_PATH, SEAM_LEAK_PATH, STRIP_PATH, UNCACHED_CAPTURE_PATH, UNREASONED_PATH,
    boot_with_render_cache_and_live, clock, dispatch_get, failed_mount_renders,
    last_failed_mount_report, last_report, private_renders, public_renders,
    public_seed_lifetime_ms, seam_control_renders, seam_leak_renders, strip_renders,
    unreasoned_renders,
};
use sha2::Digest as _;
use suprnova::StatusCode;
use suprnova::live::{
    LiveDocumentErrorKind, LiveMount, LiveMountKind, StitchFailurePolicy, StitchSlotDescriptor,
};
use suprnova::render_cache::collector::{self, Collector, current_report};
use suprnova::render_cache::live::{
    CapturedSlot, LiveDocumentFacts, StitchCapture, document_declines, record_bootstrap_nonce,
    record_document_digest, record_document_intent, record_mount, record_shell_island,
    record_stitch_capture_invalid, record_stitch_slot,
};
use suprnova::render_cache::{RenderCache, RepresentationClass};
use suprnova::view::{
    DocumentCachePolicy, DocumentResponseIntent, TrustedHtml, TrustedMarkupReason,
};
use suprnova_live::identity::{BuildId, ComponentName, ContentDigest, IslandSlot, RouteIdentity};
use suprnova_live::mount::{DocumentMountKey, MountFlags};
use suprnova_live::render_cache::composite::{
    MAX_FALLBACK_BYTES, MAX_STITCH_SLOTS, SlotFailurePolicy,
};

/// One recorded slot with every identity valid and nothing else varying but
/// the island slot and the document mount key.
fn captured(slot: &str, key: &str) -> CapturedSlot {
    CapturedSlot {
        descriptor: StitchSlotDescriptor {
            route: RouteIdentity::from_bytes(&[3u8; 32]).expect("route"),
            slot: IslandSlot::parse(slot).expect("slot"),
            document_key: DocumentMountKey::parse(key).expect("key"),
            component: ComponentName::parse("app.counter").expect("component"),
            contract_digest: ContentDigest::from_bytes(&[1u8; 32]).expect("digest"),
            protocol: 1,
            build: BuildId::parse("suprnova-1.0.0").expect("build"),
            parameters: "{}".to_owned(),
            flags: MountFlags::empty(),
            on_failure: SlotFailurePolicy::FailDocument,
        },
        html: Bytes::from_static(b"<div>island</div>"),
    }
}

#[test]
fn declines_identity_bound_islands_no_store_intents_and_deadline_free_seeds() {
    let public = LiveDocumentFacts {
        public_seed_islands: 1,
        identity_bound_islands: 0,
        seed_deadline_ms: Some(10),
        no_store: false,
        ..Default::default()
    };
    assert!(
        document_declines(Some(&public), RepresentationClass::PublicShared).is_none(),
        "a public seed with a resolved deadline stores"
    );
    let bound = LiveDocumentFacts {
        identity_bound_islands: 1,
        ..public.clone()
    };
    assert!(
        document_declines(Some(&bound), RepresentationClass::PublicShared).is_some(),
        "an identity-bound island never stores on a route that did not declare stitching"
    );
    let no_store = LiveDocumentFacts {
        no_store: true,
        ..public.clone()
    };
    assert!(
        document_declines(Some(&no_store), RepresentationClass::PublicShared).is_some(),
        "a document that declared NoStore never stores"
    );
    assert!(
        document_declines(None, RepresentationClass::PublicShared).is_none(),
        "a plain route with no Live document is left alone"
    );
    let no_deadline = LiveDocumentFacts {
        seed_deadline_ms: None,
        ..public
    };
    assert!(
        document_declines(Some(&no_deadline), RepresentationClass::PublicShared).is_some(),
        "a seed document without a resolvable deadline is not stored"
    );
}

#[test]
fn identity_bound_islands_decline_unless_the_route_is_declared_stitched() {
    let bound = LiveDocumentFacts {
        identity_bound_islands: 1,
        seed_deadline_ms: None,
        ..Default::default()
    };
    assert!(document_declines(Some(&bound), RepresentationClass::PublicShared).is_some());
    assert!(document_declines(Some(&bound), RepresentationClass::PrivateCached).is_some());
    assert!(document_declines(Some(&bound), RepresentationClass::PublicShellStitched).is_none());
    let invalid = LiveDocumentFacts {
        stitch: StitchCapture {
            invalid: true,
            ..Default::default()
        },
        ..bound.clone()
    };
    assert!(
        document_declines(Some(&invalid), RepresentationClass::PublicShellStitched).is_some(),
        "a capture that could not be represented declines even a stitched route"
    );
    let no_store = LiveDocumentFacts {
        no_store: true,
        ..bound
    };
    assert!(
        document_declines(Some(&no_store), RepresentationClass::PublicShellStitched).is_some(),
        "NoStore means this cache too, stitched or not"
    );
}

#[test]
fn a_stitch_fallback_is_bounded_at_declaration_and_never_printed() {
    let mount = LiveMount::<DogfoodCounter>::public_seed(
        "/dogfood/fallback",
        "counter",
        "dogfood-fallback",
    )
    .expect("declare mount");
    let reason = TrustedMarkupReason::new("stitch fallback test").expect("reason");
    let largest = TrustedHtml::framework_generated("x".repeat(MAX_FALLBACK_BYTES), reason.clone())
        .expect("fallback markup");
    assert!(
        mount
            .clone()
            .on_stitch_failure(StitchFailurePolicy::Fallback(largest))
            .is_ok(),
        "a fallback exactly at the stored entry's bound is accepted"
    );
    let oversized =
        TrustedHtml::framework_generated("x".repeat(MAX_FALLBACK_BYTES + 1), reason.clone())
            .expect("fallback markup");
    let Err(error) = mount.on_stitch_failure(StitchFailurePolicy::Fallback(oversized)) else {
        panic!("one byte past the bound is refused where it is declared");
    };
    assert_eq!(error.kind(), LiveDocumentErrorKind::StitchFallbackTooLarge);
    assert_eq!(
        error.to_string(),
        "live_stitch_fallback_too_large",
        "the message names the violated contract and carries no markup"
    );
    let secret = TrustedHtml::framework_generated("<b>tenant secret</b>".to_owned(), reason)
        .expect("fallback markup");
    assert_eq!(
        format!("{:?}", StitchFailurePolicy::Fallback(secret)),
        "fallback(<checked>)",
        "a policy never prints the markup it carries"
    );
    assert_eq!(format!("{:?}", StitchFailurePolicy::Omit), "omit");
    assert_eq!(
        format!("{:?}", StitchFailurePolicy::FailDocument),
        "fail_document"
    );
}

#[test]
fn a_captured_slot_never_prints_the_island_it_holds() {
    // A real identity-bound island: one principal's rendered state, with
    // that mount's signed snapshot in an attribute.
    const ISLAND: &str = "<div data-suprnova-live-root=\"counter\" \
        data-suprnova-live-snapshot=\"eyJiYWxhbmNlIjoxMjM0fQ\">balance 12.34</div>";
    let mut slot = captured("a", "doc-a");
    slot.html = Bytes::from_static(ISLAND.as_bytes());
    // The descriptor carries markup of its own: the declared fallback that
    // takes this island's place on a hit it cannot be re-rendered for.
    slot.descriptor.on_failure = SlotFailurePolicy::Fallback {
        html: "<aside>tenant fallback copy</aside>".to_owned(),
    };

    let printed = format!("{slot:?}");
    assert!(
        !printed.contains("data-suprnova-live-snapshot"),
        "a captured slot never prints the snapshot it carries: {printed}"
    );
    assert!(
        !printed.contains("balance 12.34"),
        "a captured slot never prints the island's own markup: {printed}"
    );
    assert!(
        !printed.contains("tenant fallback copy") && !printed.contains("<aside>"),
        "nor the fallback markup its descriptor carries: {printed}"
    );
    assert!(
        printed.contains("html_bytes"),
        "the length is still reported: {printed}"
    );

    // The whole reachable chain, the way a caller would hit it:
    // `current_report()` hands back a `CollectorReport` whose `Debug` is
    // derived all the way down to this slot.
    let facts = LiveDocumentFacts {
        stitch: StitchCapture {
            slots: vec![slot],
            ..Default::default()
        },
        ..Default::default()
    };
    let printed = format!("{facts:?}");
    assert!(
        !printed.contains("data-suprnova-live-snapshot")
            && !printed.contains("balance 12.34")
            && !printed.contains("tenant fallback copy"),
        "nothing above the slot un-redacts it: {printed}"
    );
}

#[tokio::test]
async fn capture_records_slots_shell_islands_nonce_and_digest_in_order() {
    let facts = Collector::scope(async {
        record_mount(LiveMountKind::PublicSeed, Some(5_000));
        record_shell_island(
            &IslandSlot::parse("seed").expect("slot"),
            &DocumentMountKey::parse("doc-seed").expect("key"),
        );
        record_bootstrap_nonce(Some("n0nce"));
        record_mount(LiveMountKind::IdentityBound, None);
        record_stitch_slot(captured("a", "doc-a"));
        record_mount(LiveMountKind::IdentityBound, None);
        record_stitch_slot(captured("b", "doc-b"));
        record_document_digest([7u8; 32]);
        current_report()
            .expect("report")
            .live_document
            .expect("facts")
    })
    .await;
    assert_eq!(facts.identity_bound_islands, 2);
    assert_eq!(facts.stitch.slots.len(), 2);
    assert_eq!(facts.stitch.slots[0].descriptor.slot.as_str(), "a");
    assert_eq!(facts.stitch.slots[1].descriptor.slot.as_str(), "b");
    assert_eq!(facts.stitch.shell_islands.len(), 1);
    assert_eq!(facts.stitch.shell_islands[0].slot, "seed");
    assert_eq!(facts.stitch.nonce.as_deref(), Some("n0nce"));
    assert_eq!(facts.stitch.document_digest, Some([7u8; 32]));
    assert!(!facts.stitch.invalid);
}

#[tokio::test]
async fn a_second_digest_a_conflicting_nonce_or_too_many_slots_marks_the_capture_invalid() {
    let facts = Collector::scope(async {
        record_document_digest([1u8; 32]);
        record_document_digest([2u8; 32]);
        current_report()
            .expect("report")
            .live_document
            .expect("facts")
    })
    .await;
    assert!(
        facts.stitch.invalid,
        "two rendered documents in one request"
    );
    let facts = Collector::scope(async {
        record_bootstrap_nonce(Some("one"));
        record_bootstrap_nonce(Some("two"));
        current_report()
            .expect("report")
            .live_document
            .expect("facts")
    })
    .await;
    assert!(facts.stitch.invalid, "two bootstraps with different nonces");
    let facts = Collector::scope(async {
        for index in 0..=MAX_STITCH_SLOTS {
            record_stitch_slot(captured(&format!("s{index}"), &format!("k{index}")));
        }
        current_report()
            .expect("report")
            .live_document
            .expect("facts")
    })
    .await;
    assert!(facts.stitch.invalid, "a 33rd slot");
    assert_eq!(
        facts.stitch.slots.len(),
        MAX_STITCH_SLOTS,
        "the slot past the bound is not recorded either"
    );
}

#[tokio::test]
async fn an_island_that_cannot_be_described_as_a_slot_invalidates_the_capture() {
    let facts = Collector::scope(async {
        record_mount(LiveMountKind::IdentityBound, None);
        // What `LiveDocument::mount` records when the island's parameters
        // cannot be spelled as a canonical document within the slot's bound:
        // the island still rendered, so the document is served, but no shell
        // can be cut from it.
        record_stitch_capture_invalid();
        current_report()
            .expect("report")
            .live_document
            .expect("facts")
    })
    .await;
    assert!(facts.stitch.invalid);
    assert!(
        facts.stitch.slots.is_empty(),
        "an island with no describable slot is never recorded as one"
    );
    assert!(document_declines(Some(&facts), RepresentationClass::PublicShellStitched).is_some());
}

#[tokio::test]
#[serial_test::serial]
async fn an_identity_bound_mount_captures_its_exact_island_bytes_inside_a_slot_scope() {
    let harness = boot_with_render_cache_and_live().await;
    let login = dispatch_get(&harness, DOCUMENT_PATH, &[("x-test-login", "user-7")]).await;
    let cookie = login.session_cookie();
    let response = dispatch_get(
        &harness,
        CAPTURE_PATH,
        &[("x-test-login", "user-7"), ("cookie", &cookie)],
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let report = last_report().expect("the capture handler stored its report");
    let facts = report.live_document.clone().expect("facts");
    assert_eq!(facts.stitch.slots.len(), 1);
    let html = std::str::from_utf8(&facts.stitch.slots[0].html).expect("utf8");
    assert!(
        html.starts_with("<div data-suprnova-live-root=\"counter\""),
        "{html}"
    );
    assert!(
        std::str::from_utf8(&response.body)
            .expect("utf8")
            .contains(html),
        "the island bytes appear verbatim in the document"
    );
    assert!(
        report.slot_reads > 0,
        "the mount's own reads were attributed to the slot"
    );
    assert!(
        !report.context.principal_read,
        "the handler itself read no principal"
    );
    assert!(
        report.gate.context.principal_read,
        "the auth guard's read is a gate read"
    );
    assert_eq!(
        facts.stitch.nonce.as_deref(),
        Some(CAPTURE_NONCE),
        "the bootstrap's own nonce is recorded once"
    );
    assert!(
        std::str::from_utf8(&response.body)
            .expect("utf8")
            .contains(&format!("nonce=\"{CAPTURE_NONCE}\"")),
        "the recorded nonce is the one the emitted script elements carry"
    );
    let expected: [u8; 32] = sha2::Sha256::digest(&response.body).into();
    assert_eq!(
        facts.stitch.document_digest,
        Some(expected),
        "the recorded digest covers exactly the bytes the document served"
    );
}

/// `LiveDocument` builds a stitch descriptor, copies each island's markup,
/// and hashes the rendered body only when a collector is active, because
/// every `record_*` those feed is a no-op without one. This route carries no
/// render cache policy, so nothing wraps its handler in a scope: the
/// document still renders exactly as it does under one, and the handler sees
/// no report to store.
#[tokio::test]
#[serial_test::serial]
async fn a_document_rendered_outside_a_collector_scope_records_nothing() {
    let harness = boot_with_render_cache_and_live().await;
    let login = dispatch_get(&harness, DOCUMENT_PATH, &[("x-test-login", "user-7")]).await;
    let cookie = login.session_cookie();
    let response = dispatch_get(
        &harness,
        UNCACHED_CAPTURE_PATH,
        &[("x-test-login", "user-7"), ("cookie", &cookie)],
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let body = std::str::from_utf8(&response.body).expect("utf8");
    // The handler ran and produced a whole Live document: the island the
    // guarded copy would have captured is in the served bytes, and the
    // bootstrap stamped the nonce the guarded recording would have read.
    assert!(
        body.contains("<div data-suprnova-live-root=\"counter\""),
        "the island is rendered into the document unchanged: {body}"
    );
    assert!(
        body.contains(&format!("nonce=\"{CAPTURE_NONCE}\"")),
        "and the bootstrap stamped its nonce"
    );
    // Which means the `None` below is what the handler saw, not a handler
    // that never ran: it stores the report unconditionally before returning.
    assert!(
        last_report().is_none(),
        "no collector was active, so no LiveDocumentFacts were recorded"
    );
}

#[tokio::test]
async fn mount_facts_accumulate_across_multiple_mounts_in_one_request() {
    Collector::scope(async {
        record_mount(LiveMountKind::PublicSeed, Some(500));
        record_mount(LiveMountKind::PublicSeed, Some(200));
        record_mount(LiveMountKind::IdentityBound, None);
        let facts = current_report()
            .expect("collector active")
            .live_document
            .expect("facts recorded");
        assert_eq!(facts.public_seed_islands, 2, "counts add across mounts");
        assert_eq!(
            facts.identity_bound_islands, 1,
            "counts add across mount kinds too"
        );
        assert_eq!(
            facts.seed_deadline_ms,
            Some(200),
            "the deadline takes the minimum of every mounted seed's own deadline"
        );
    })
    .await;
}

#[tokio::test]
async fn a_no_store_document_intent_is_sticky_once_recorded() {
    Collector::scope(async {
        record_mount(LiveMountKind::PublicSeed, Some(500));
        let no_store = DocumentResponseIntent::html(StatusCode::OK)
            .expect("intent")
            .with_cache(DocumentCachePolicy::NoStore);
        record_document_intent(&no_store);
        let public = DocumentResponseIntent::html(StatusCode::OK)
            .expect("intent")
            .with_cache(DocumentCachePolicy::Public);
        record_document_intent(&public);
        let facts = current_report()
            .expect("collector active")
            .live_document
            .expect("facts recorded");
        assert!(
            facts.no_store,
            "no_store stays set once any document in the request declared it, \
             even if a later document in the same request did not"
        );
    })
    .await;
}

/// Also the end-to-end guard that `EntryHeader::seed_deadline_ms` is
/// actually carried by a stored entry, and not left `None`: the deadline is
/// recorded by the collector, read off the report by `lead_render`, and
/// handed to `entry_header`; every freshness decision afterwards reads it
/// back out of the stored header. The entry below is a hit right up to the
/// deadline and renders again the moment it is past, which no other field
/// of the header could produce - a time-fresh entry with no seed deadline
/// stored would still be a hit on the third dispatch.
#[tokio::test]
#[serial_test::serial]
async fn a_public_seed_document_is_a_hit_until_its_seed_deadline() {
    let harness = boot_with_render_cache_and_live().await;
    let first = dispatch_get(&harness, DOCUMENT_PATH, &[]).await;
    assert_eq!(
        first.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&first.body)
    );
    assert_eq!(public_renders(), 1, "the first GET renders");
    // Also proves R86: a demoted `private, ...` header (the pre-fix
    // behavior on this exact route, since `DocumentResponseIntent::html()`
    // defaults to `Private`) would never contain `s-maxage` at all.
    let cache_control = first.header("cache-control").expect("cache-control");
    assert!(
        cache_control.starts_with("public,"),
        "a public-seed document is not demoted to a private class: {cache_control}"
    );
    let max_age: u64 = cache_control
        .split("max-age=")
        .nth(1)
        .expect("max-age")
        .split(',')
        .next()
        .expect("max-age value")
        .parse()
        .expect("seconds");
    assert!(
        max_age * 1_000 <= public_seed_lifetime_ms(&harness),
        "max-age never outlives the seed: {max_age}"
    );

    let second = dispatch_get(&harness, DOCUMENT_PATH, &[]).await;
    assert_eq!(second.status, StatusCode::OK);
    assert_eq!(
        public_renders(),
        1,
        "the second GET is served from the cache, not a render"
    );

    clock(&harness).advance_ms(public_seed_lifetime_ms(&harness) + 1);
    let expired = dispatch_get(&harness, DOCUMENT_PATH, &[]).await;
    assert_eq!(expired.status, StatusCode::OK);
    assert_eq!(
        public_renders(),
        2,
        "past the seed deadline the entry is dead and a fresh document renders"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn an_identity_bound_dashboard_is_never_stored() {
    let harness = boot_with_render_cache_and_live().await;
    // Sign in on one request, as a login handler would, so the
    // identity-bound render on the next request binds the session that
    // survives the framework's fixation rotation (the same sequence
    // `live_public_seed_actions.rs`'s own identity-bound test uses).
    let login = dispatch_get(&harness, DOCUMENT_PATH, &[("x-test-login", "user-7")]).await;
    assert_eq!(
        login.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&login.body)
    );
    let cookie = login.session_cookie();

    let first = dispatch_get(
        &harness,
        PRIVATE_DOCUMENT_PATH,
        &[("x-test-login", "user-7"), ("cookie", &cookie)],
    )
    .await;
    assert_eq!(
        first.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&first.body)
    );
    assert_eq!(private_renders(), 1, "the first GET renders");

    let again = dispatch_get(
        &harness,
        PRIVATE_DOCUMENT_PATH,
        &[("x-test-login", "user-7"), ("cookie", &cookie)],
    )
    .await;
    assert_eq!(
        again.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&again.body)
    );
    // The route declares `Principal` variance, so `classify`'s own
    // key/value guard is satisfied for two requests from the same user and
    // cannot be what declines the second one (see finding 4): whatever
    // declines it here is `document_declines`'s identity-bound branch and
    // nothing else.
    assert_eq!(
        private_renders(),
        2,
        "an identity-bound document is re-rendered every time, never a hit"
    );
    assert!(
        again.header("age").is_none(),
        "an identity-bound document is never served from a stored entry"
    );
    assert!(
        again.header("etag").is_none(),
        "an identity-bound document was never stored, so it has no validator"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn an_identity_bound_mount_declines_even_when_the_handler_never_calls_render() {
    let harness = boot_with_render_cache_and_live().await;
    let login = dispatch_get(&harness, DOCUMENT_PATH, &[("x-test-login", "user-7")]).await;
    let cookie = login.session_cookie();

    let first = dispatch_get(
        &harness,
        RAW_PATH,
        &[("x-test-login", "user-7"), ("cookie", &cookie)],
    )
    .await;
    assert_eq!(
        first.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&first.body)
    );
    assert_eq!(private_renders(), 1, "the first GET renders");

    let again = dispatch_get(
        &harness,
        RAW_PATH,
        &[("x-test-login", "user-7"), ("cookie", &cookie)],
    )
    .await;
    assert_eq!(again.status, StatusCode::OK);
    // R87: the fact is recorded at `mount`, not at `render` - this route's
    // handler never calls `render` at all, so if the fact were recorded
    // there instead (the brief's original placement), `report.live_document`
    // would be `None` here and this identity-bound mount would be stored
    // and served like any other `PrivateCached` entry with satisfied
    // variance.
    assert_eq!(
        private_renders(),
        2,
        "an identity-bound mount declines even when render is never called"
    );
    assert!(again.header("etag").is_none());
}

#[tokio::test]
#[serial_test::serial]
async fn a_declared_private_cached_route_with_no_identity_read_is_still_cached() {
    // R89: `UNREASONED_PATH` declares `PrivateCached` with `Principal`
    // variance and reads no identity in its handler. `classify` starts
    // from the declared class and only narrows further, so this always
    // produces `(PrivateCached, [])` on every request - a shape the R86
    // invariant must not decline, since the declared class already forced
    // `Principal` variance (Task 14 round 6) and the key is already
    // partitioned by the resolved principal before the render begins.
    let harness = boot_with_render_cache_and_live().await;
    let login = dispatch_get(&harness, DOCUMENT_PATH, &[("x-test-login", "user-7")]).await;
    let cookie = login.session_cookie();

    let first = dispatch_get(
        &harness,
        UNREASONED_PATH,
        &[("x-test-login", "user-7"), ("cookie", &cookie)],
    )
    .await;
    assert_eq!(
        first.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&first.body)
    );
    assert_eq!(unreasoned_renders(), 1, "the first GET renders");

    let again = dispatch_get(
        &harness,
        UNREASONED_PATH,
        &[("x-test-login", "user-7"), ("cookie", &cookie)],
    )
    .await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(
        unreasoned_renders(),
        1,
        "a declared-PrivateCached route whose handler reads no identity is a hit for the \
         same signed-in principal, not permanently uncacheable"
    );
    assert!(
        again.header("etag").is_some(),
        "a stored entry carries a validator"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn a_class_narrowed_with_no_attached_reason_is_declined() {
    // Finding 8: `STRIP_PATH` declares `PublicShared` with no variance and
    // reads an identity, which `classify` would normally narrow to
    // `PrivateCached` with `ClassificationReason::PrincipalObserved`
    // attached. The handler immediately strips that reason via the
    // test-only seam, producing exactly the shape
    // `is_unreasoned_private_class`'s call site exists to catch: a class
    // genuinely narrowed away from the declared one, with no reason left
    // for the value guard to check the key against. Since nothing
    // partitions the key here, storing this would let every caller share
    // one entry keyed to nobody in particular.
    let harness = boot_with_render_cache_and_live().await;
    // A session cookie is carried on both requests: without one, every
    // response carries a fresh `Set-Cookie` and is ineligible for storage
    // regardless of classification, which would make this test pass
    // vacuously.
    let login = dispatch_get(&harness, DOCUMENT_PATH, &[("x-test-login", "user-7")]).await;
    let cookie = login.session_cookie();
    let first = dispatch_get(
        &harness,
        STRIP_PATH,
        &[("x-test-login", "user-7"), ("cookie", &cookie)],
    )
    .await;
    assert_eq!(
        first.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&first.body)
    );
    assert_eq!(strip_renders(), 1, "the first GET renders");

    let again = dispatch_get(
        &harness,
        STRIP_PATH,
        &[("x-test-login", "user-7"), ("cookie", &cookie)],
    )
    .await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(
        strip_renders(),
        2,
        "a class narrowed with no attached reason must be declined, never stored"
    );
    assert!(again.header("etag").is_none());
}

#[tokio::test]
#[serial_test::serial]
async fn the_seam_can_only_decline_never_serve_one_user_another_users_body() {
    // Finding 10 / R90: on a route declared `PrivateCached` with `Tenant`
    // variance only (no `Principal` dimension in the key - R89 exempts the
    // declared class from the invariant), a `PrincipalObserved` reason with
    // no `Principal` dimension declared must decline via the pre-existing
    // value guard whether or not the test-only seam is called. If the seam
    // ever touched the real classification the value guard checks, calling
    // it would blank that guard's input and let `user-9` be served the
    // body rendered for `user-7`.
    let harness = boot_with_render_cache_and_live().await;
    let login7 = dispatch_get(&harness, DOCUMENT_PATH, &[("x-test-login", "user-7")]).await;
    let cookie7 = login7.session_cookie();
    let login9 = dispatch_get(&harness, DOCUMENT_PATH, &[("x-test-login", "user-9")]).await;
    let cookie9 = login9.session_cookie();

    // Control: identical policy and handler shape, no seam call. Confirms
    // the value guard alone already declines this shape, so any difference
    // measured on the leak route below is attributable to the seam.
    let c1 = dispatch_get(
        &harness,
        SEAM_CONTROL_PATH,
        &[("x-test-login", "user-7"), ("cookie", &cookie7)],
    )
    .await;
    assert_eq!(
        c1.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&c1.body)
    );
    let _c2 = dispatch_get(
        &harness,
        SEAM_CONTROL_PATH,
        &[("x-test-login", "user-9"), ("cookie", &cookie9)],
    )
    .await;
    assert_eq!(
        seam_control_renders(),
        2,
        "without the seam, the value guard declines a Principal reason with no Principal \
         dimension declared"
    );

    // Leak shape: identical except the handler calls the seam.
    let l1 = dispatch_get(
        &harness,
        SEAM_LEAK_PATH,
        &[("x-test-login", "user-7"), ("cookie", &cookie7)],
    )
    .await;
    assert_eq!(
        l1.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&l1.body)
    );
    let l2 = dispatch_get(
        &harness,
        SEAM_LEAK_PATH,
        &[("x-test-login", "user-9"), ("cookie", &cookie9)],
    )
    .await;
    assert_eq!(l2.status, StatusCode::OK);
    assert_eq!(
        seam_leak_renders(),
        2,
        "the seam must only ever cause the invariant to decline, never weaken the value \
         guard that runs after it"
    );
    assert!(l2.header("etag").is_none(), "never stored, so no validator");
    assert_ne!(
        l1.body, l2.body,
        "user-9 must never be served the body rendered for user-7"
    );
}

/// The one residual path of the slot-read exemption Task 8 introduced in
/// `collector::mark_incomplete`: a slot read the collector cannot name
/// marks only that slot, not the whole report, and the safety argument
/// rests on `live::document_declines` refusing any route that keeps an
/// identity-bound island without stitching.
///
/// `LiveDocument::mount` records the identity-bound fact *after* the
/// mount, so a mount that fails propagates with `?` before
/// `render_cache::live::record_mount` ever runs. Nothing then tells
/// `document_declines` there was an identity-bound island at all, and a
/// report whose only unnameable read happened inside that failed mount is
/// still storable. This test states exactly what happens on that path and
/// why it is safe:
///
/// - **What is asserted:** the response is *published*, not declined, and
///   what is published contains no island bytes - no Live root element, no
///   signed snapshot, nothing the failed mount would have produced.
/// - **Why that is the safe outcome:** the island bytes are the only thing
///   a stored shell could carry that was derived from one visitor's
///   identity, and a mount that failed produced none. There is nothing
///   identity-derived in the stored representation to serve to anybody
///   else, so storing it shares nothing that was not already public.
#[tokio::test]
#[serial_test::serial]
async fn a_failed_identity_bound_mount_publishes_a_shell_with_no_island_bytes() {
    let harness = boot_with_render_cache_and_live().await;

    // 1. Anonymous, and the route carries no `AuthMiddleware`, so nothing
    //    records principal evidence and the identity-bound mount refuses.
    //    The handler answers 200 all the same - a handler that turned the
    //    failure into a 500 would be declined by eligibility on status
    //    alone and would never reach the path under test.
    let first = dispatch_get(&harness, FAILED_MOUNT_PATH, &[]).await;
    assert_eq!(
        first.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&first.body)
    );
    assert_eq!(failed_mount_renders(), 1);
    let body = String::from_utf8(first.body.to_vec()).expect("utf8");
    assert!(body.contains(FAILED_MOUNT_FALLBACK), "{body}");

    // 2. The mount fact was never recorded, so `document_declines` has
    //    nothing to decline the route for. This is the residual path
    //    itself, stated as the report the middleware actually saw.
    let report = last_failed_mount_report().expect("the handler stored its report");
    assert!(
        report.live_document.is_none(),
        "the `?` on the failed mount returned before `record_mount` ran"
    );
    assert!(
        document_declines(
            report.live_document.as_ref(),
            RepresentationClass::PublicShared
        )
        .is_none(),
        "with no recorded island there is nothing for the Live decline to fire on"
    );

    // 3. So the render really is published, and the published bytes carry
    //    no island content: the second request is a hit (the handler is
    //    never reached again) whose body is the same island-free shell.
    let second = dispatch_get(&harness, FAILED_MOUNT_PATH, &[]).await;
    assert_eq!(second.status, StatusCode::OK);
    assert_eq!(
        failed_mount_renders(),
        1,
        "the second request is answered from the stored entry"
    );
    assert_eq!(second.body, first.body, "byte for byte the stored entry");
    for marker in [
        "data-suprnova-live-root",
        "data-suprnova-live-snapshot",
        "data-suprnova-live-document-key",
    ] {
        assert!(
            !body.contains(marker),
            "the stored shell carries no island bytes, so it carries no {marker}: {body}"
        );
    }
    let stored = RenderCache::inspect_route_for_test(FAILED_MOUNT_PATH)
        .await
        .expect("the entry is reachable under the route's own lookup key");
    assert_eq!(stored.class, RepresentationClass::PublicShared);
    assert_eq!(stored.status, 200);
    assert_eq!(
        stored.slots, 0,
        "a Complete entry with no stitch slots: there was no island to cut one for"
    );

    // 4. And an unnameable read inside that failed mount would not have
    //    changed any of it. This is the exemption's own arithmetic, replayed
    //    in the exact order `LiveDocument::mount` produces it on the failing
    //    path: the mount runs inside `slot_scope`, its unnameable read is
    //    counted as a slot read and marks nothing, and `record_mount` never
    //    runs because the `?` returned first.
    let replayed = Collector::scope(async {
        collector::begin_handler();
        collector::slot_scope(async {
            collector::observe_unobservable_read();
        })
        .await;
        collector::observe_table_read("posts");
        current_report().expect("report")
    })
    .await;
    assert_eq!(replayed.slot_reads, 1);
    assert!(
        !replayed.context.overflowed,
        "a slot read the collector cannot name marks only that slot, not the report"
    );
    assert!(
        replayed.storable().is_some(),
        "so the shell stays storable - which is safe only because it holds no island bytes"
    );
    assert!(replayed.live_document.is_none());
}
