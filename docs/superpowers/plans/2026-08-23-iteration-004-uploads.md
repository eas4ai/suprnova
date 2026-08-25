# Iteration 004 Uploads Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver secure, resumable-within-the-current-document Live uploads with opaque handles, secret grants, bounded chunk transfer, quarantine/validation/scanning, deliberate authorized finalization, accessible browser controls, and provider-neutral direct-transfer conformance.

**Architecture:** Keep upload control authority on the server and file bytes out of Live snapshots. A revisioned upload record owns the state machine and idempotency outcomes. The engine's reference reverse-proxy provider owns path policy, hashing, and state while delegating raw asynchronous quarantine I/O through `QuarantineStore`; test support supplies the Tokio filesystem adapter. A provider-neutral direct-transfer trait and in-memory conformance adapter prove the alternate contract without claiming a vendor integration. The optional browser artifact owns selected `File` objects, grants, chunks, progress, interruption, and keyed-morph continuity only for the current document. Finalization is an explicit Live action boundary and never an automatic consequence of transfer completion.

**Tech Stack:** Rust 1.91.1, serde/serde_json canonical codecs, HMAC/HKDF purpose-separated signing, bytes 1.11.1, exact `imagesize` 0.15.0 with only PNG/JPEG/GIF/WebP features, Tokio in test support only, strict TypeScript 6.0.3, native File/Blob/fetch/AbortController APIs, Vitest/fast-check, Playwright, deterministic filesystem and provider test doubles.

---

## Dependencies and execution rules

- This is Plan 2 of 4. Complete `2026-08-23-iteration-004-shared-foundation.md` first.
- Execute this plan before Plan 3 in the shared Iteration 004 worktree. The plans
  touch common metadata, host, lifecycle, fixture, package, and test-support files
  and must not run concurrently. Plan 4 begins only after both pass in order.
- Work only in `/home/shawn/workspace2/suprnova-live/.worktrees/iteration-004-uploads-async` and never push without explicit authorization.
- Start every shell command with `rtk`; use `apply_patch` for hand edits; do not use blanket `-D warnings`.
- Use test-owned temporary directories and injected clocks/randomness. Never recursively remove an unresolved or broad path.
- Do not add browser persistence, concrete cloud-vendor code, framework adapters, RenderCache, or component-library work.

## File structure

### Create

- `src/upload/mod.rs`
- `src/upload/identity.rs`
- `src/upload/protocol.rs`
- `src/upload/state.rs`
- `src/upload/ledger.rs`
- `src/upload/provider.rs`
- `src/upload/quarantine.rs`
- `src/upload/direct_provider.rs`
- `src/upload/validation.rs`
- `src/upload/finalize.rs`
- `src/upload/service.rs`
- `src/upload/cleanup.rs`
- `src/upload/telemetry.rs`
- `tests/upload_identity.rs`
- `tests/upload_protocol.rs`
- `tests/upload_state.rs`
- `tests/upload_file_provider.rs`
- `tests/upload_direct_provider.rs`
- `tests/upload_validation.rs`
- `tests/upload_finalization.rs`
- `tests/upload_cleanup.rs`
- `tests/upload_security.rs`
- `browser/src/uploads/feature.ts`
- `browser/src/uploads/types.ts`
- `browser/src/uploads/manager.ts`
- `browser/src/uploads/transfer.ts`
- `browser/src/uploads/progress.ts`
- `browser/src/uploads/morph.ts`
- `browser/src/uploads/resume.ts`
- `browser/tests/upload-manager.test.ts`
- `browser/tests/upload-transfer.test.ts`
- `browser/tests/upload-progress.test.ts`
- `browser/tests/upload-morph.test.ts`
- `browser/tests/upload-resume.test.ts`
- `browser/e2e/uploads.spec.ts`
- `fuzz/fuzz_targets/upload_control.rs`
- `fuzz/fuzz_targets/upload_transition.rs`
- `fuzz/fuzz_targets/upload_media_header.rs`
- `crates/suprnova-live-test-support/src/file_quarantine_store.rs`

### Modify

- `src/lib.rs`
- `src/error.rs`
- `src/limits.rs`
- `src/host/capabilities.rs`
- `src/host/context.rs`
- `src/metadata/field.rs`
- `src/metadata/digest.rs`
- `src/metadata/component.rs`
- `src/resource/{cancel,owner,queue}.rs`
- `Cargo.toml`
- `Cargo.lock`
- `THIRD_PARTY_LICENSES.md`
- `browser/src/entry-uploads-esm.ts`
- `browser/src/entry-uploads-classic.ts`
- `browser/src/runtime/diagnostics.ts`
- `browser/src/signals/lifecycle.ts`
- `browser/src/feedback/*`
- `browser/src/morph/*`
- `browser/test-host/server.mjs`
- `browser/test-host/scenarios.mjs`
- `crates/suprnova-live-test-support/src/lib.rs`
- `crates/suprnova-live-test-support/src/host.rs`
- `crates/suprnova-live-test-support/Cargo.toml`
- `fuzz/Cargo.toml`

## Task 1: Define opaque upload identity and secret transfer grants

**Files:** `src/upload/{mod,identity}.rs`, `src/lib.rs`, `tests/upload_identity.rs`, `tests/upload_security.rs`

- [ ] Add failing tests for typed handle parsing, purpose-separated grant signing, expiry/scope checks, cross-upload/principal/tenant reuse, and a sentinel scan across every public/debug/serialized representation:

  ```rust
  #[test]
  fn handle_is_non_authority_and_grant_is_secret() {
      let issued = fixture_issuer().issue(scope(), descriptor()).unwrap();
      assert!(!issued.handle().as_str().contains(GRANT_SENTINEL));
      assert!(!format!("{:?}", issued).contains(GRANT_SENTINEL));
      assert_eq!(verify_handle_only(issued.handle()), Err(UploadError::GrantRequired));
      assert_eq!(fixture_verifier().verify(issued.grant(), wrong_tenant()), Err(UploadError::ScopeMismatch));
  }
  ```

- [ ] Run `rtk cargo test --test upload_identity --test upload_security`; record failure because upload identity types are absent.
- [ ] Implement opaque and secret types with redacted `Debug`, no `Display` for grants, zeroization, bounded base64url decoding, and claims bound to handle/component/field/principal/session/tenant/expiry/protocol:

  ```rust
  #[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
  #[serde(transparent)]
  pub struct UploadHandle(Uuid);

  #[derive(Clone, Zeroize, ZeroizeOnDrop)]
  pub struct TransferGrant(Zeroizing<Vec<u8>>);

  pub struct VerifiedTransferGrant {
      handle: UploadHandle,
      component: ComponentName,
      field: ModelField,
      scope: HostScopeFacts,
      expires_at: UnixMillis,
      upload_protocol: u16,
  }
  ```

  Derive grant keys from the engine key with HKDF-SHA-256 purpose `suprnova-live/upload-grant/v1`, sign canonical claims with HMAC-SHA-256, and reject noncanonical/unknown fields before claims allocation.

- [ ] Export `pub mod upload;`, run identity/security/error-redaction tests, format, and Clippy.
- [ ] Commit: `feat(upload): separate handles from transfer grants`.

## Task 2: Implement bounded upload codecs and the revisioned state machine

**Files:** `src/upload/{protocol,state}.rs`, `src/limits.rs`, `src/error.rs`, protocol/state tests, existing v4 upload fixtures

- [ ] Add failing fixture/property tests for every operation and state, duplicate keys, unknown major versions, malformed transitions, cross-handle chunks, oversize fields, and reordered/duplicate calls:

  ```rust
  proptest! {
      #[test]
      fn accepted_transitions_never_regress(sequence in transition_sequence()) {
          let mut state = UploadState::created();
          let mut revision = UploadRevision::initial();
          for command in sequence {
              if let Ok(next) = state.apply(revision, command) {
                  prop_assert!(next.revision() > revision);
                  prop_assert!(next.state().rank() >= state.rank());
                  state = next.state().clone();
                  revision = next.revision();
              }
          }
      }
  }
  ```

- [ ] Run `rtk cargo test --test upload_protocol --test upload_state`; record failure because codecs and transitions are absent.
- [ ] Implement independent upload protocol v1 and the closed state machine:

  ```rust
  pub const SUPPORTED_UPLOAD_PROTOCOL_VERSIONS: &[u16] = &[1];

  pub enum UploadState {
      Created,
      Queued,
      Transferring,
      Verifying,
      Ready,
      Finalizing,
      Finalized,
      Rejected,
      Canceled,
      Expired,
      Failed,
  }

  pub enum UploadOperation {
      Create(CreateUpload),
      PutChunk(PutChunk),
      Status(StatusUpload),
      Complete(CompleteUpload),
      Cancel(CancelUpload),
      Reacquire(ReacquireUpload),
  }

  pub enum UploadTransition {
      Queue,
      BeginTransfer,
      PutChunk(AcceptedChunk),
      Complete,
      Accept,
      BeginFinalize,
      CommitFinalize,
      Cancel,
      Reject,
      Expire,
      Fail,
  }
  ```

  `UploadOperation` is the independently versioned external wire vocabulary in
  `operations`/`codec_cases`; `UploadTransition` is the internal ledger
  vocabulary in `transition_cases`. Preserve all checked-in fixture bytes. Bind
  them through one exhaustive service mapping: create establishes `Created` then
  queues; first chunk begins transfer then records `PutChunk`; complete enters
  verifying and later validation chooses `Accept` or `Reject`; cancel maps to
  `Cancel`; status and reacquire do not transition. Finalize actions map to
  `BeginFinalize` and `CommitFinalize`; cleanup maps to `Expire`. A compile-time
  provider/host failure maps to `Fail`, making the locked `failed` terminal state
  reachable without changing the existing fixture bytes. A compile-time
  exhaustive match and fixture test fail when either layer gains an unmapped
  variant.

  Decode through a bounded JSON object walker before constructing commands.
  Every mutating command carries expected revision plus idempotency key; terminal
  duplicates return the stored outcome, stale alternatives return
  `UploadConflict`, and no operation can transition backward. `Reacquire` remains
  a server-side service operation invoked by an authenticated application route;
  it does not authorize or register a reserved Live endpoint.

- [ ] Add upload-specific limits for counts, per-file/aggregate/chunk/in-flight bytes, concurrency, rates, retries, age, validation/scanning time, storage, and cleanup batches. Run fixtures, properties, security tests, and protocol v1/v2 regression suites.
- [ ] Commit: `feat(upload): add bounded protocol and state machine`.

## Task 3: Add conditional upload authority and service admission

**Files:** `src/upload/{ledger,service}.rs`, host capabilities/context, test-support host, ledger/service tests

- [ ] Add failing concurrency tests proving one committed outcome per upload revision, duplicate outcome replay, failed claim behavior, current authorization on every control boundary, and bounded creation rate:

  ```rust
  #[tokio::test]
  async fn concurrent_completion_accepts_one_revision() {
      let service = fixture_service();
      let (left, right) = tokio::join!(service.complete(request(7, "a")), service.complete(request(7, "b")));
      assert_eq!([left, right].iter().filter(|result| result.is_ok()).count(), 1);
      assert_eq!(service.record().revision(), UploadRevision::new(8).unwrap());
  }
  ```

- [ ] Run `rtk cargo test --test upload_state upload_service`; record the missing ledger/service failure.
- [ ] Define a host-neutral conditional ledger and capability port:

  ```rust
  pub trait UploadLedger: Send + Sync {
      fn load<'a>(&'a self, handle: &'a UploadHandle) -> UploadFuture<'a, Result<Option<UploadRecord>, UploadError>>;
      fn transition<'a>(&'a self, request: ConditionalTransition) -> UploadFuture<'a, Result<TransitionOutcome, UploadError>>;
  }

  pub trait UploadAuthorizationPort: Send + Sync {
      fn authorize<'a>(&'a self, request: UploadAuthorizationRequest<'a>)
          -> UploadFuture<'a, Result<UploadAuthorizationDecision, UploadError>>;
  }
  ```

  Add the port to trusted host capabilities. `UploadService` verifies request
  authenticity, grant, scope, expiry, current principal/policy, limits,
  transition, and idempotency in that order; expensive provider work begins only
  after admission. Service lifetime and queued work use the shared
  `ResourceOwner`/`ResourceQueue`; concurrent transfer admission uses
  `PermitPool`; cancellation uses `CancellationFlag`. Extend those foundation
  types only when a missing primitive is proved by a failing foundation test.

- [ ] Implement a complete in-memory/reference ledger in test support and run state/service concurrency plus hostile-context suites.
- [ ] Commit: `feat(upload): add conditional control authority`.

## Task 4: Stream chunks into the quarantined file provider

**Files:** `src/upload/{provider,quarantine}.rs`, `src/resource/*`,
`crates/suprnova-live-test-support/{Cargo.toml,src/file_quarantine_store.rs}`,
provider tests and test-support fixtures

- [ ] Add failing tests using test-owned temporary roots for short reads/writes, duplicate chunks, checksum mismatch, interrupted streams, descriptor limits, process recovery, disk-full/provider failure, shutdown, and path traversal:

  ```rust
  #[tokio::test]
  async fn file_provider_never_derives_paths_from_client_names() {
      let provider = fixture_file_provider();
      let mut chunk = body(b"safe");
      provider.write_chunk(write("../../served.html", 0), &mut chunk).await.unwrap();
      let stored = provider.inspect(handle()).await.unwrap();
      assert!(stored.path().starts_with(provider.quarantine_root()));
      assert!(!stored.path().to_string_lossy().contains("served.html"));
  }
  ```

- [ ] Run `rtk cargo test --test upload_file_provider`; record failure because no provider exists.
  - [ ] Define the executor-neutral streaming provider and raw quarantine I/O
        capability:

    ```rust
    pub trait QuarantineStore: Send + Sync {
        fn create_exclusive<'a>(
            &'a self,
            object: &'a QuarantineObject,
        ) -> UploadFuture<'a, Result<(), UploadError>>;
        fn write_at<'a>(
            &'a self,
            object: &'a QuarantineObject,
            offset: u64,
            bytes: &'a [u8],
        ) -> UploadFuture<'a, Result<(), UploadError>>;
        fn sync<'a>(
            &'a self,
            object: &'a QuarantineObject,
        ) -> UploadFuture<'a, Result<(), UploadError>>;
        fn read_at<'a>(
            &'a self,
            object: &'a QuarantineObject,
            offset: u64,
            maximum_bytes: usize,
        ) -> UploadFuture<'a, Result<Bytes, UploadError>>;
        fn read_prefix<'a>(
            &'a self,
            object: &'a QuarantineObject,
            maximum_bytes: usize,
        ) -> UploadFuture<'a, Result<Bytes, UploadError>>;
        fn remove<'a>(
            &'a self,
            object: &'a QuarantineObject,
        ) -> UploadFuture<'a, Result<RemoveDisposition, UploadError>>;
    }

    pub trait UploadProvider: Send + Sync {
        fn prepare<'a>(&'a self, request: PrepareTransfer<'a>) -> UploadFuture<'a, Result<TransferPlan, UploadError>>;
        fn verify<'a>(&'a self, request: VerifyTransfer<'a>) -> UploadFuture<'a, Result<IntegrityEvidence, UploadError>>;
        fn cancel<'a>(&'a self, handle: &'a UploadHandle) -> UploadFuture<'a, Result<(), UploadError>>;
        fn cleanup<'a>(&'a self, handle: &'a UploadHandle) -> UploadFuture<'a, Result<(), UploadError>>;
    }

    pub trait ReverseProxyUploadProvider: UploadProvider {
        fn write_chunk<'a>(&'a self, request: WriteChunk<'a>, body: &'a mut dyn ChunkBody)
            -> UploadFuture<'a, Result<ChunkReceipt, UploadError>>;
    }

    pub trait DirectUploadProvider: UploadProvider {
        fn report_part<'a>(&'a self, request: ReportDirectPart<'a>)
            -> UploadFuture<'a, Result<ChunkReceipt, UploadError>>;
    }
    ```

    `read_at` is the bounded primitive required for engine-owned streaming
    whole-file hashing and recovery; `read_prefix` is the inspection-oriented
    convenience used by later validation. `QuarantinedFileProvider<S:
QuarantineStore>` stays in the engine. It creates
    server-random `QuarantineObject` names, owns path policy, chunk/whole-file
    hashing, revision state, a shared `ResourceOwner`, descriptor/chunk
    `PermitPool`s, `CancellationFlag`, and at most two chunk buffers per active
    transfer. It calls `sync` before readiness and never exposes a filesystem path
    or serving root. No engine code calls blocking `std::fs` or depends on Tokio.

    `TokioFileQuarantineStore` lives only in `suprnova-live-test-support`, maps
    opaque objects beneath one pre-opened test-owned root, uses `create_new`,
    bounded `tokio::fs::File` handles, positional writes, `sync_data`, prefix
    reads, and idempotent removal. Add exact Tokio 1.53.1 `fs`/`io-util` features
    to the test-support crate, not the production engine.

- [ ] Run provider tests under controlled shutdown/fault schedules, security boundaries, and file-descriptor/memory limit assertions.
- [ ] Commit: `feat(upload): add quarantined file transfer provider`.

## Task 5: Prove the provider-neutral direct-transfer contract

**Files:** `src/upload/direct_provider.rs`, `tests/upload_direct_provider.rs`, test-support adapter

- [ ] Add a failing shared conformance suite that runs against the file provider and a direct-transfer reference adapter:

  ```rust
  pub async fn assert_provider_conformance(factory: impl ProviderFactory) {
      let plan = factory.provider().prepare(prepare()).await.unwrap();
      assert!(plan.instructions().all(|instruction| instruction.is_constrained()));
      assert_duplicate_part_is_idempotent(&factory).await;
      assert_cross_upload_part_is_rejected(&factory).await;
      assert_completion_requires_integrity(&factory).await;
      assert_cancel_expire_cleanup_are_idempotent(&factory).await;
  }
  ```

- [ ] Run `rtk cargo test --test upload_direct_provider`; record failure because direct instructions are not modeled.
- [ ] Implement provider-neutral constrained instructions and an in-memory conformance adapter:

  ```rust
  pub struct DirectTransferInstruction {
      method: TransferMethod,
      endpoint: TrustedProviderUrl,
      required_headers: BoundedHeaders,
      part: UploadPart,
      expires_at: UnixMillis,
      maximum_bytes: usize,
  }
  ```

  The server retains handle/state authority; instructions are short-lived, part-bound, method-bound, byte-bound, and provider-origin-bound. Completion imports provider integrity evidence, then performs the same verification/state transitions as the file provider. Name the adapter `DirectProviderConformanceAdapter`; do not call it S3 or vendor-ready.

  The conformance adapter issues at most one part instruction at a time and
  returns the next only after importing the preceding provider outcome. An
  opaque direct-part reference binds that report to the upload and range but is
  explicitly non-authoritative; endpoint and header credentials remain
  redacted and expired instructions are renewed only inside the temporary
  upload's independent lifetime. Provider-mode operations are separate typed
  extension traits, so a direct adapter cannot accidentally accept a
  reverse-proxy request (or vice versa) through a runtime mode switch.

- [ ] Run shared provider conformance, malformed instruction tests, and grant/URL leak tests.
- [ ] Commit: `feat(upload): prove direct provider conformance`.

## Task 6: Validate, scan, and deliberately finalize uploads

**Files:** `src/upload/{validation,finalize}.rs`, metadata modules,
`Cargo.toml`, `Cargo.lock`, `THIRD_PARTY_LICENSES.md`,
`fuzz/fuzz_targets/upload_media_header.rs`, validation/finalization tests

- [ ] Add failing tests for filename/MIME/extension disagreement, actual byte size/hash, bounded image/media headers, application rules, scan allow/reject/timeout/unavailable policy, unauthorized finalize, provider/database failures, retry, compensation, and reconciliation:

  ```rust
  #[tokio::test]
  async fn ready_content_is_not_durable_until_authorized_finalize() {
      let service = ready_upload_service();
      assert_eq!(service.public_location(handle()).await, None);
      assert_eq!(service.finalize(denied_finalize()).await.unwrap_err(), UploadError::Unauthorized);
      assert_eq!(service.record().state(), &UploadState::Ready);
  }
  ```

  - [ ] Run validation/finalization tests; record failure because verification/finalization services are absent.
  - [ ] Add the exact production dependency with the reviewed minimal feature set:

    ```toml
    imagesize = { version = "=0.15.0", default-features = false, features = ["gif", "jpeg", "png", "webp"] }
    ```

    Provenance reviewed 2026-08-24: upstream is
    `https://github.com/Roughsketch/imagesize`; published 0.15.0 is MIT licensed,
    has no normal transitive dependencies, and exposes format features so unused
    parsers stay out. No matching RustSec advisory was found in the official
    advisory database at review time. Confirm the checked package checksum through
    `Cargo.lock`, run `rtk cargo tree -e normal -i imagesize`, regenerate
    `THIRD_PARTY_LICENSES.md` with
    `rtk node scripts/generate-license-inventory.mjs`, and prove the repository
    MSRV before acceptance. Point-in-time advisory absence is not a substitute for
    the repository's release audit.

  - [ ] Implement validation and scanning ports over authoritative quarantined bytes:

  ```rust
  pub trait UploadScanner: Send + Sync {
      fn scan<'a>(&'a self, input: ScanInput<'a>) -> UploadFuture<'a, Result<ScanDisposition, UploadError>>;
  }

  pub enum ScanDisposition { Clean, Rejected(ScanReason), Unavailable }

    pub trait UploadFinalizer: Send + Sync {
      fn prepare<'a>(&'a self, request: FinalizeRequest<'a>) -> UploadFuture<'a, Result<PreparedFinalize, UploadError>>;
      fn commit<'a>(&'a self, prepared: PreparedFinalize) -> UploadFuture<'a, Result<DurableUpload, UploadError>>;
      fn compensate<'a>(&'a self, failed: FailedFinalize) -> UploadFuture<'a, Result<(), UploadError>>;
    }
  ```

  Add a dimension-only `MediaHeaderProbe` over `QuarantineStore::read_prefix`.
  PNG reads at most 32 bytes, GIF 16 bytes, WebP 64 bytes, and JPEG 256 KiB;
  larger or truncated headers fail closed as `MediaHeaderUnproved`. Call
  `imagesize::blob_size` only after magic-byte classification and the applicable
  prefix cap, then reject zero dimensions, integer overflow, and declared
  width/height/pixel limits. Never decode pixels in the engine.

  Finalize rechecks principal/session/tenant/component/field/policy/revision/readiness, records one logical idempotency outcome, and exposes reconciliation for partially committed provider/database work. Documentation and errors promise neither distributed atomicity nor exactly-once external effects.

  - [ ] Add `upload_media_header.rs` fuzzing arbitrary capped bytes across the four
        enabled formats with no panic, allocation escape, loop escape, or dimension
        overflow. Persist malformed JPEG marker chains and truncated WebP/PNG/GIF
        regressions. Add digest-significant upload field metadata for count,
        replacement, accepted types, dimension/pixel limits, scan policy, and finalize
        action. Run metadata, validation, finalization, fuzz-build, license, MSRV, and
        action-regression tests.

- [ ] Commit: `feat(upload): validate and finalize quarantined content`.

## Task 7: Implement race-safe expiry and cleanup

**Files:** `src/upload/{cleanup,telemetry}.rs`, `tests/upload_cleanup.rs`

- [ ] Add failing controlled-clock tests for cancel/remove/expire races against transfer, verify, scan, and finalize; orphan retry; bounded batches; and unrelated-scope availability:

  ```rust
  #[tokio::test(start_paused = true)]
  async fn cleanup_cannot_delete_a_committed_finalize() {
      let fixture = finalize_cleanup_race();
      let (finalized, cleaned) = tokio::join!(fixture.finalize(), fixture.cleanup());
      assert!(finalized.is_ok() || cleaned.unwrap().disposition() == CleanupDisposition::Deferred);
      assert_ne!(fixture.record().state(), &UploadState::Failed);
  }
  ```

- [ ] Run `rtk cargo test --test upload_cleanup`; record failure because cleanup leases are absent.
- [ ] Implement conditional cleanup claims and bounded telemetry:

  ```rust
  pub struct CleanupPolicy {
      pub batch_items: NonZeroUsize,
      pub batch_bytes: NonZeroUsize,
      pub lease: Duration,
      pub retry: BoundedBackoff,
  }

  pub struct CleanupMetrics {
      pub age_bucket: UploadAgeBucket,
      pub volume_bucket: UploadVolumeBucket,
      pub outcome: CleanupOutcome,
      pub retry_bucket: RetryBucket,
      pub orphaned: bool,
  }
  ```

  Provider deletion and ledger terminalization are idempotent. Cleanup batches
  use the shared `BoundedQueue`/`ResourceQueue`, cancellation uses
  `CancellationFlag`, and concurrent deletion is admitted through the shared
  `PermitPool`; do not introduce an upload-private queue or semaphore. Browser
  cooperation is never required. Metrics contain buckets and outcomes only,
  never handles, filenames, paths, topics, principals, grants, or raw errors.

- [ ] Run cleanup, concurrency, security, telemetry-cardinality, and controlled-shutdown tests.
- [ ] Commit: `feat(upload): add bounded cleanup reconciliation`.

## Task 8: Implement current-document upload management in the optional artifact

**Files:** `browser/src/uploads/{types,feature,manager,transfer,resume}.ts`, upload entry points, browser unit tests

- [ ] Add failing tests for single/multiple selection, multiple fields, replacement, repeated selection, zero-byte files, directory/path oddities, offline interruption, bounded concurrency/chunks, cancel/retry/remove, typed upload-handle proposal/clear, application-owned reacquisition, and no browser persistence calls:

  ```ts
  it("retains file and grant only inside the current document owner", async () => {
    const stores = instrumentAmbientStorage(window);
    const manager = fixtureUploadManager({
      maxActive: 4,
      chunkBytes: 256 * 1024,
    });
    await manager.select(input, [fileOfSize(16 * MIB)]);
    expect(manager.snapshot().active[0]?.retainedChunks).toBeLessThanOrEqual(2);
    expect(stores.calls()).toEqual([]);
    manager.dispose();
    expect(manager.inspectSecrets()).toEqual({
      grants: 0,
      files: 0,
      chunks: 0,
    });
  });
  ```

- [ ] Run `rtk npm --prefix browser test -- upload-manager.test.ts upload-transfer.test.ts upload-resume.test.ts`; record failure because the upload feature is inert.
- [ ] Implement one manager per document and one transfer owner per selected file:

  ```ts
  export interface ActiveUpload {
    readonly handle: UploadHandle;
    readonly grant: SecretTransferGrant;
    readonly file: File;
    readonly idempotencyKey: string;
    readonly chunks: ChunkMap;
    readonly abort: AbortController;
  }

  export class UploadManager {
    readonly #owner = new BoundedOwner<QueuedUpload>({
      maxItems: 64,
      maxBytes: 256 * 1024,
      maxActive: 4,
    });
  }

  export interface UploadApplicationPort {
    reacquire(
      request: Readonly<{
        field: string;
        fileIdentity: UploadFileIdentity;
        handle: UploadHandle;
      }>,
    ): Promise<ReacquiredUpload>;
  }
  ```

  Build the manager on the shared browser `BoundedOwner`; do not introduce a
  second queue/permit implementation. Slice at configured 256 KiB, retain at
  most two chunk buffers per active transfer, use injected
  transport/connectivity/randomness, and send grants only in authorization
  headers or bodies that never enter URL/history/diagnostics. After create,
  call `island.proposeUploadHandle(field, handle)`; after remove, cancel, expiry,
  or rejected replacement, call it with `null`. Core rejects undeclared fields,
  malformed handles, cross-island use, and retired islands, and the next
  deliberate Live action obtains the proposal through the existing model batch.

  Reload has no resume state. `reacquire(handle)` exists only through the
  optional `UploadApplicationPort` supplied by application bootstrap and still
  requires the user-held `File` to match authoritative identity.
  `ReacquiredUpload` carries both the authoritative uploaded-byte offset and
  next chunk index; status reconciliation returns the same cursor, and the
  browser never derives the index from its current chunk-size configuration.
  Fence every pending reacquisition with a bounded island/field generation so
  newer selection/removal, island retirement, or document disposal discards a
  late grant without installing it. The feature
  contains no fixed reacquisition URL and registers no `/__live/` reacquire
  route; the reference application demonstrates an authenticated route outside
  that namespace.

- [ ] Register the real feature from both upload entry points. Run manager/transfer/resume, lifecycle, diagnostics, and optional-artifact budget tests.
- [ ] Commit: `feat(browser): transfer bounded current-document uploads`.

## Task 9: Add truthful accessible progress and keyed-morph continuity

**Files:** `browser/src/uploads/{progress,morph}.ts`, feedback/signals/morph hooks, progress/morph tests, Playwright upload spec

- [x] Add failing DOM tests for every visible state, numeric bounds, announcement throttling, keyboard controls, error association, reduced motion, compatible keyed preservation, rekey/removal/navigation/bfcache disposal, empty-string clearing, and inability to assign a non-empty file value, `files`, or path:

  ```ts
  expect(progressRoot.getAttribute("data-live-upload-state")).toBe("verifying");
  expect(progressRoot.getAttribute("aria-busy")).toBe("true");
  expect(progressRoot.getAttribute("aria-valuenow")).toBe("100");
  expect(input.files?.item(0)).toBe(selectedFile);
  const writes = observeFileInputWrites(input);
  morphWithDifferentUploadKey();
  expect(input.files?.length).toBe(0);
  expect(writes).toEqual([{ property: "value", value: "" }]);
  expect(transfer.disposeCount).toBe(1);
  ```

- [x] Run focused Vitest and Chromium Playwright upload tests; record missing progress/morph behavior.
- [x] Implement semantic projection through the existing signal/feedback contracts:

  ```ts
  export type UploadPresentationState =
    | "queued"
    | "transferring"
    | "verifying"
    | "ready"
    | "finalizing"
    | "finalized"
    | "interrupted"
    | "failed"
    | "canceled"
    | "expired";

  export interface UploadProgressView {
    readonly state: UploadPresentationState;
    readonly loadedBytes: number;
    readonly totalBytes: number;
    readonly percent: number | null;
  }
  ```

  Preserve only when island identity, upload field, keyed input, active handle,
  and progress/control roots are compatible. Removal or replacement may assign
  only `input.value = ""` to clear the retired native selection. Never assign a
  non-empty `input.value`, `input.files`, path text, or ownership to a replacement
  island. Announce state changes at a bounded cadence while controls remain
  keyboard-native.

- [x] Run upload unit tests and Playwright Chromium/Firefox/WebKit with accessibility/CSP checks and deterministic lifecycle events.
- [x] Commit: `feat(browser): preserve accessible upload continuity`.

## Task 10: Fuzz, verify, and hand off uploads

**Files:** upload fuzz targets and every upload file

- [x] Add fuzz targets that decode arbitrary bytes under strict limits, apply arbitrary transition sequences, and probe capped PNG/JPEG/GIF/WebP headers without panic, allocation/loop escape, dimension overflow, state regression, or cross-handle acceptance:

  ```rust
    fuzz_target!(|input: &[u8]| {
      let limits = UploadCodecLimits::hostile_test();
      if let Ok(command) = decode_upload_command(input, limits) {
          assert!(command.encoded_len() <= limits.max_bytes());
      }
    });

    fuzz_target!(|input: &[u8]| {
        let capped = &input[..input.len().min(MAX_MEDIA_HEADER_BYTES)];
        let _ = MediaHeaderProbe::hostile_test().probe(capped);
    });
  ```

- [x] Run the complete upload gate:

  ```bash
  rtk cargo fmt --all -- --check
  rtk env CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --all-features
  rtk env CARGO_INCREMENTAL=0 cargo test --test upload_identity --test upload_protocol --test upload_state --test upload_file_provider --test upload_direct_provider --test upload_validation --test upload_finalization --test upload_cleanup --test upload_security
  rtk cargo +nightly fuzz build
  rtk cargo +nightly fuzz run upload_protocol -- -runs=1000
  rtk cargo +nightly fuzz run upload_state -- -runs=1000
  rtk cargo +nightly fuzz run upload_media_header -- -runs=1000
  rtk npm --prefix browser run format:check
  rtk npm --prefix browser run lint
  rtk npm --prefix browser run typecheck
  rtk npm --prefix browser test -- upload-manager.test.ts upload-transfer.test.ts upload-progress.test.ts upload-morph.test.ts upload-resume.test.ts
  rtk npm --prefix browser run test:browser -- --project=chromium uploads.spec.ts
  rtk npm --prefix browser run build:check
  rtk npm --prefix browser run budget
  rtk git diff --check
  ```

- [x] Inspect logs, traces, snapshots, HTML, URLs/history, diagnostics, serialized actions/models, and test-host inspection output with the grant sentinel. Confirm quarantined/unverified content has no serving route and reload has no ambient resume state.
- [x] Commit verification corrections as `chore: close iteration 004 upload gate`.

## Definition-of-done coverage

- DOD 2–4: Tasks 1–3 cover identity, grants, independent codecs, revisions, idempotency, and hostile input.
- DOD 5–7: Tasks 3–5 cover real bounded file transfer, provider-neutral direct conformance, and enforced quotas.
- DOD 8–10: Tasks 6–7 cover authoritative validation/scanning, explicit finalization, compensation, races, expiry, and cleanup.
- DOD 11–14: Tasks 8–9 cover native selection, progress/accessibility, keyed morph behavior, current-document resume, and explicit reacquisition.
- DOD 25, 29–32: Tasks 8–10 prepare real-host/browser/security/fuzz coverage and the U4/16 behavior consumed by Plan 4.

## Plan self-review checklist

- [x] No grant type implements revealing formatting or serialization into Live state.
- [x] No client filename/path influences a storage path and quarantine is never served.
- [x] File and direct providers share state/authority semantics; no vendor capability is claimed.
- [x] Ready and finalized remain distinct and only an authorized deliberate action finalizes.
- [x] Resume is current-document-only unless explicit authenticated reacquisition succeeds.
- [x] Every queue, buffer, permit, file descriptor, task, timer, listener, and transfer has one bounded owner and idempotent retirement.
