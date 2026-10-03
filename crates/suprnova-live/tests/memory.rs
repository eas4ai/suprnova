//! Heap tests for the memory-footprint commitment (MEM-003) in the engine:
//! each measures what one operation allocates, with dhat counting every
//! allocation in this process. Every test holds one lock, so one profiler
//! runs at a time; the mechanism runs each test in a process of its own.

mod component_support;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use bytes::Bytes;
use component_support::{
    FailurePoint, FixtureControl, ManualClock, SNAPSHOT_PADDING, SequenceGenerator, install,
    key_ring, metadata, trusted_context,
};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::identity::{ContentDigest, KeyId, RouteIdentity, UnixMillis};
use suprnova_live::ledger::{LedgerLimits, MemoryInstanceLedger};
use suprnova_live::limits::InputLimits;
use suprnova_live::mount::{
    DocumentMountKey, DocumentMountScope, MountFlags, MountLimits, MountProviders,
    PrivateMountRequest, PrivateMountService,
};
use suprnova_live::registry::{ComponentDescriptor, ComponentRegistryBuilder};
use suprnova_live::render_cache::RepresentationClass;
use suprnova_live::render_cache::composite::{
    CompositeEntry, CompositeHeader, HeaderPiece, HeaderTemplate, Segment, SegmentGraph,
    ShellIsland, SlotFailurePolicy, StitchSlot, surrounding_digest,
};
use suprnova_live::render_cache::entry::{EntryHeader, SafeHeaders};
use suprnova_live::render_cache::generation::GenerationSet;
use suprnova_live::render_cache::key::RenderKey;
use suprnova_live::render_cache::variance::VarianceDescriptor;
use suprnova_live::snapshot::{MountedDocumentPath, SnapshotLimits};
use suprnova_live::view::{RenderLimits, ViewRenderer};

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

async fn exclusive() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
}

fn heap() -> dhat::HeapStats {
    dhat::HeapStats::get()
}

fn mount_service() -> PrivateMountService {
    let clock = Arc::new(ManualClock::new(1_000));
    let registry = ComponentRegistryBuilder::new()
        .register(ComponentDescriptor::with_hooks(
            metadata().clone(),
            install(FixtureControl::new(FailurePoint::None)),
        ))
        .expect("component registers")
        .build();
    let ledger = Arc::new(MemoryInstanceLedger::new(
        clock.clone(),
        LedgerLimits::new(100, 10_000, 4, 64).expect("ledger limits"),
    ));
    PrivateMountService::new(
        MountProviders::new(
            Arc::new(registry),
            ledger,
            clock,
            Arc::new(SequenceGenerator::new(0x20)),
            Arc::new(key_ring()),
        ),
        // Room for a state string of a few kilobytes, which the shared
        // fixture's limits refuse.
        SnapshotLimits::new(
            InputLimits::new(64 * 1024, 4, 64, 16 * 1024).expect("input limits"),
            50,
            10_000,
            20_000,
            8,
            8,
        )
        .expect("snapshot limits"),
        ViewRenderer::new(RenderLimits::standard()).expect("render limits"),
        MountLimits::new(1_000, 3, 512 * 1024, 8).expect("mount limits"),
    )
    .expect("mount service")
}

/// The bytes one private mount allocates with a snapshot padded by
/// `padding`, and the length of the snapshot it signs.
async fn mount_cost(service: &PrivateMountService, key: &str, padding: usize) -> (u64, usize) {
    SNAPSHOT_PADDING.store(padding, Ordering::SeqCst);
    let context = trusted_context();
    let mut document = DocumentMountScope::new();
    let request = PrivateMountRequest::new(
        DocumentMountKey::parse(key).expect("document mount key"),
        CanonicalValue::Object(BTreeMap::new()),
        MountFlags::empty(),
    );
    let before = heap().total_bytes;
    let output = service
        .mount(&mut document, request, &context)
        .await
        .expect("a private mount");
    let used = heap().total_bytes - before;
    (used, output.metadata().signed_snapshot().len())
}

/// MEM-003: a private mount does not copy the snapshot it signed: what it
/// allocates grows with the snapshot by less than one more copy of it.
#[tokio::test]
async fn mem_audit_a_mount_does_not_copy_its_signed_snapshot() {
    let _lock = exclusive().await;
    let service = mount_service();
    let _profiler = dhat::Profiler::builder().testing().build();
    mount_cost(&service, "warm-up", 2_300).await;
    let (small, small_len) = mount_cost(&service, "small", 2_300).await;
    let (large, large_len) = mount_cost(&service, "large", 3_100).await;
    let slope = (large - small) as f64 / (large_len - small_len) as f64;
    assert!(
        slope < MOUNT_BYTES_PER_SNAPSHOT_BYTE,
        "a mount allocates {slope:.2} bytes per snapshot byte"
    );
}

/// The allocation per signed snapshot byte a private mount may make. The
/// same mounts measured 64.78 when the mount copied the signed snapshot
/// twice, one byte per copy; the bound sits between that and the 62.78 a
/// mount that copies it no more allocates, and any new per-byte copy
/// crosses it.
const MOUNT_BYTES_PER_SNAPSHOT_BYTE: f64 = 63.8;

fn keys() -> SnapshotKeyRing {
    let active = KeyRecord::new(
        KeyId::parse("render-cache-test").expect("key id"),
        RootKey::new(vec![3; 32]).expect("root key"),
        UnixMillis::new(0),
        UnixMillis::new(u64::MAX / 2),
        UnixMillis::new(u64::MAX),
    )
    .expect("key record");
    SnapshotKeyRing::new(active, Vec::new()).expect("key ring")
}

fn composite_entry(keys: &SnapshotKeyRing) -> CompositeEntry {
    let head = b"<!doctype html><html><body>".to_vec();
    let tail = b"</body></html>".to_vec();
    let shell = Bytes::from([head.as_slice(), tail.as_slice()].concat());
    let slot = |index: u8| StitchSlot {
        route: RouteIdentity::from_bytes(&[4u8 + index; 32])
            .expect("route")
            .to_base64url(),
        slot: format!("counter-{index}"),
        document_key: format!("doc-counter-{index}"),
        component: "app.counter".to_owned(),
        contract_digest: ContentDigest::from_bytes(&[2u8; 32])
            .expect("digest")
            .to_base64url(),
        protocol: 1,
        build: "suprnova-1.0.0".to_owned(),
        parameters: format!("{{\"filter\":\"{}\",\"page\":{index}}}", "f".repeat(512)),
        flags: BTreeMap::new(),
        on_failure: SlotFailurePolicy::FailDocument,
        surrounding: String::new(),
    };
    let mut graph = SegmentGraph {
        segments: vec![
            Segment::Literal {
                len: head.len() as u32,
            },
            Segment::Slot { index: 0 },
            Segment::Slot { index: 1 },
            Segment::Nonce,
            Segment::Literal {
                len: tail.len() as u32,
            },
        ],
        slots: vec![slot(0), slot(1)],
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
    for index in 0..2 {
        graph.slots[index].surrounding =
            surrounding_digest(&graph, &shell, index).expect("surrounding");
    }
    let header = EntryHeader {
        key: RenderKey::for_test(keys, "/composite"),
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
            ("cache-control", "private"),
        ])
        .expect("safe"),
        content_encoding: None,
    };
    CompositeEntry::new(header, graph, shell).expect("composite entry")
}

/// MEM-003: an entry's canonical header bytes are serialized from the
/// entry as it is, allocating no more than serializing an owned header.
#[tokio::test]
async fn mem_audit_composite_header_bytes_do_not_clone_the_graph() {
    let _lock = exclusive().await;
    let entry = composite_entry(&keys());
    let owned = CompositeHeader {
        entry: entry.header().clone(),
        graph: entry.graph().clone(),
    };
    let _profiler = dhat::Profiler::builder().testing().build();
    let max = 64 * 1024;
    owned.canonical_bytes(max).expect("a warm-up");
    let before = heap();
    let from_owned = owned.canonical_bytes(max).expect("owned bytes");
    let after = heap();
    let from_entry = entry.canonical_header_bytes(max).expect("entry bytes");
    let last = heap();
    assert_eq!(from_owned, from_entry);
    let owned_blocks = after.total_blocks - before.total_blocks;
    let entry_blocks = last.total_blocks - after.total_blocks;
    let owned_bytes = after.total_bytes - before.total_bytes;
    let entry_bytes = last.total_bytes - after.total_bytes;
    assert!(
        entry_blocks <= owned_blocks && entry_bytes <= owned_bytes,
        "the entry allocated {entry_blocks} blocks and {entry_bytes} bytes, \
         an owned header {owned_blocks} and {owned_bytes}"
    );
}

/// MEM-003: validating a document path allocates only the owned path.
#[tokio::test]
async fn mem_audit_a_document_path_is_validated_without_copies() {
    let _lock = exclusive().await;
    let _profiler = dhat::Profiler::builder().testing().build();
    let before = heap().total_blocks;
    let path = MountedDocumentPath::parse("/docs/guide/intro").expect("a path");
    let used = heap().total_blocks - before;
    assert_eq!(path.as_str(), "/docs/guide/intro");
    assert_eq!(
        used, 1,
        "parsing a three-segment path made {used} allocations"
    );
}
