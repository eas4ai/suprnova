# Suprnova Live Iteration 001 Trusted Interaction Spine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the complete iteration 001 trusted interaction spine: bounded and signed seed/instance snapshots, deterministic hydration/dehydration, Tier 0 revision authority and promotion, versioned request/response contracts, shared Rust/TypeScript conformance data, hostile-input coverage, and the release gate and performance evidence required by the confirmed scope contract.

**Architecture:** Build one Rust 2024 internal engine crate with dependency-inverted clocks, randomness, and ledger traits. External bytes cross a bounded duplicate-rejecting parser into typed canonical values; only verified snapshot capabilities can reach hydration or promotion. Snapshot bodies are RFC 8785 canonical JSON, keyed by explicit purpose/version derivation and wrapped in HMAC proofs. The Tier 0 ledger stores only scoped concurrency/idempotency metadata, never component state. A strict TypeScript package independently validates the same checked-in golden fixtures and response state-machine outcomes. This repository remains standalone and does not depend on or modify Suprnova or Magnetar.

**Tech Stack:** Rust 2024/MSRV 1.91.1; Serde; `serde_json_canonicalizer`; RustCrypto HKDF/HMAC/SHA-256; `base64`; `zeroize`; `getrandom`; `thiserror`; `async-trait`; Tokio test support; Proptest; cargo-fuzz; strict TypeScript 6.0.x; ESLint 10 with `typescript-eslint`; Prettier 3; Vitest 4; npm lockfile.

---

## Locked implementation decisions

- The crate package is named `suprnova-live`, but it remains an internal engine. Application-facing re-exports under `suprnova::live` wait for iteration 007.
- The crate uses edition 2024 and `rust-version = "1.91.1"`, matching the active Suprnova workspace. `rust-toolchain.toml` pins the same compiler with `rustfmt` and `clippy`.
- Signed bodies use RFC 8785 canonical JSON. JSON numbers are limited to interoperable finite IEEE-754 values. Revisions, Unix milliseconds, generations, and other potentially 64-bit counters use validated decimal strings on the wire so JavaScript cannot silently round them.
- Snapshot envelopes have a canonical signed `body` and a fixed-length base64url-without-padding `signature`. `key_id` lives inside the signed body, preventing key-selection substitution.
- Purpose/version-specific 32-byte MAC keys derive from at-least-32-byte root keys with HKDF-SHA-256. Seed-v1 and instance-v1 have distinct context strings. HMAC verification uses RustCrypto's constant-time verifier.
- Browser-proposed nonces are 16-32 bytes after base64url decoding. They are identity input only. The ledger obtains the authoritative instance ID from an injected server-side generator.
- Tier 0 claims the successor revision before uncoupled action work. An abandoned or expired claim consumes the instance and requires refresh. Only a successfully committed claim records an accepted outcome. The guarantee is one committed accepted outcome per base revision, never exactly-once invocation or external effects.
- A retry can recover an existing promotion only when scope, seed digest, nonce, and idempotency identity all match. Any other nonce reuse is rejected; a replay with a fresh nonce creates a separately scoped instance subject to limits.
- `LiveError` exposes stable category, recovery, and safe detail code. Its production `Display`/`Debug` paths never include snapshot bytes, signatures, keys, state, or arbitrary hostile strings.
- Golden fixture JSON is the cross-language source of truth. Rust and TypeScript read the same files; neither keeps a duplicate expected-value table.
- The browser directory in iteration 001 is a conformance package, not the Live browser runtime promised by iteration 003.

## Task 1: Create the reproducible Rust and TypeScript workspace

**Files:**

- Create: `Cargo.toml`
- Create: `Cargo.lock`
- Create: `rust-toolchain.toml`
- Create: `LICENSE`
- Create: `THIRD_PARTY_LICENSES.md`
- Create: `src/lib.rs`
- Create: `tests/workspace_contract.rs`
- Create: `browser/package.json`
- Create: `browser/package-lock.json`
- Create: `browser/tsconfig.json`
- Create: `browser/tsconfig.build.json`
- Create: `browser/eslint.config.mjs`
- Create: `browser/.prettierrc.json`
- Create: `browser/.gitignore`
- Create: `browser/src/index.ts`
- Create: `browser/tests/workspace-contract.test.ts`
- Create: `scripts/generate-license-inventory.mjs`

- [x] Add only the build scaffold required to run a test: exact package metadata, Rust 2024/MSRV, empty documented module facade, strict TypeScript compiler configuration, pinned development tools, and MIT license inventory. Do not add snapshot/protocol behavior yet.
- [x] Write `tests/workspace_contract.rs` first to assert the internal crate version constants, supported snapshot/protocol version constants, and absence of default feature drift. Run `rtk env CARGO_INCREMENTAL=0 cargo test --test workspace_contract`; observe the expected unresolved/missing-constant failure.
- [x] Add the minimal version constants and documented facade needed for the Rust test to pass. Run the same test and `rtk env CARGO_INCREMENTAL=0 cargo check --all-targets --all-features`.
- [x] Write `browser/tests/workspace-contract.test.ts` first to import the conformance package/version constants and assert protocol/snapshot v1. Run `(cd browser && npm test -- --run tests/workspace-contract.test.ts)`; observe the expected missing export failure.
- [x] Add the minimal TypeScript exports. Run the targeted test, `(cd browser && npm run typecheck)`, and `(cd browser && npm run build)`.
- [x] Generate exact lockfiles with Cargo 1.91.1 and npm 11.3.0. Document every resolved direct/transitive license in generated `THIRD_PARTY_LICENSES.md`; `scripts/generate-license-inventory.mjs --check` fails lockfile or license drift.
- [x] Commit: `build: establish Suprnova Live iteration 001 workspace`.

The initial Rust facade is intentionally small:

```rust
#![forbid(unsafe_code)]
#![deny(missing_docs, rustdoc::broken_intra_doc_links)]

/// Snapshot schema versions supported by iteration 001.
pub const SUPPORTED_SNAPSHOT_VERSIONS: &[u16] = &[1];
/// Wire protocol versions supported by iteration 001.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[u16] = &[1];
```

## Task 2: Implement bounded canonical data and identity primitives

**Files:**

- Create: `src/limits.rs`
- Create: `src/identity.rs`
- Create: `src/canonical/mod.rs`
- Create: `src/canonical/value.rs`
- Create: `src/canonical/parser.rs`
- Create: `src/canonical/serializer.rs`
- Create: `src/error.rs`
- Create: `tests/canonical_contract.rs`
- Create: `tests/canonical_properties.rs`
- Create: `tests/error_redaction.rs`

- [x] Write failing tests for maximum input bytes, maximum nesting, maximum collection entries, duplicate object keys, unknown/non-finite or non-interoperable numbers, malformed UTF-8, invalid identifiers, and stable safe recovery categories. Run `rtk env CARGO_INCREMENTAL=0 cargo test --test canonical_contract --test error_redaction`; verify failures are missing behavior rather than broken test setup.
- [x] Add `InputLimits`, validated identifier newtypes, decimal-string `Revision`/`UnixMillis`/`Generation`, base64url byte identities, `LiveError`, `ErrorCategory`, `RecoveryInstruction`, and redacted `SafeDiagnosticCode`.
- [x] Implement a duplicate-aware Serde visitor for `CanonicalValue`. Reject the byte limit before parsing; count nesting and entries during the visitor before pushing into collections. Do not parse hostile arbitrary state directly into `serde_json::Value`.
- [x] Implement canonical serialization through `serde_json_canonicalizer` only after the value/profile validator succeeds. Add the RFC 8785 numeric normalization and UTF-16 property-order examples from the pinned standard reference.
- [x] Add property tests for supported values: `parse(canonicalize(value)) == value` and repeated canonicalization produces identical bytes.
- [x] Run the targeted tests and `rtk env CARGO_INCREMENTAL=0 cargo clippy --all-targets --all-features`; refactor only while green.
- [x] Commit: `feat: add bounded canonical value contracts`.

Core boundary shape:

```rust
pub struct InputLimits {
    pub max_bytes: usize,
    pub max_depth: usize,
    pub max_entries: usize,
    pub max_string_bytes: usize,
}

pub enum CanonicalValue {
    Null,
    Bool(bool),
    Number(CanonicalNumber),
    String(String),
    Array(Vec<CanonicalValue>),
    Object(BTreeMap<String, CanonicalValue>),
}

pub struct LiveError {
    category: ErrorCategory,
    recovery: RecoveryInstruction,
    detail: SafeDiagnosticCode,
    source: Option<Box<dyn Error + Send + Sync>>,
}
```

## Task 3: Implement purpose-separated snapshot keys and signatures

**Files:**

- Create: `src/crypto/mod.rs`
- Create: `src/crypto/key.rs`
- Create: `src/crypto/key_ring.rs`
- Create: `src/crypto/signature.rs`
- Create: `tests/crypto_contract.rs`
- Create: `tests/crypto_rfc_vectors.rs`

- [x] Write failing tests using RFC 5869 and RFC 4231 vectors, seed/instance purpose separation, wrong purpose, wrong key, malformed signature length, unknown key ID, not-yet-active key, overlap acceptance, retired-key rejection, and weak root-key rejection.
- [x] Implement zeroizing root-key storage, explicit `KeyId`, activation/retirement windows, one active signing key, bounded verification-key count, and configuration validation that fails closed.
- [x] Derive keys with fixed domain salt and purpose/schema-specific HKDF info. Sign canonical body bytes with HMAC-SHA-256 and emit exactly 32 signature bytes as base64url without padding.
- [x] Verify selection and time window before calling constant-time `verify_slice`; never compare MAC bytes with ordinary equality. Ensure error formatting does not include key IDs supplied by hostile input unless converted to a bounded safe digest.
- [x] Run the targeted tests on both the default toolchain and MSRV: `rtk env CARGO_INCREMENTAL=0 cargo test --test crypto_contract --test crypto_rfc_vectors` and `rtk env CARGO_INCREMENTAL=0 cargo +1.91.1 test --test crypto_contract --test crypto_rfc_vectors`.
- [x] Commit: `feat: add snapshot key derivation and signing`.

The derivation contexts are versioned constants, not caller strings:

```rust
enum SnapshotPurpose {
    SeedV1,
    InstanceV1,
}

const HKDF_SALT_V1: &[u8] = b"suprnova-live/snapshot-hkdf/v1";
const SEED_INFO_V1: &[u8] = b"suprnova-live/seed-signature/v1";
const INSTANCE_INFO_V1: &[u8] = b"suprnova-live/instance-signature/v1";
```

## Task 4: Implement signed seed and instanced snapshot schemas

**Files:**

- Create: `src/snapshot/mod.rs`
- Create: `src/snapshot/error.rs`
- Create: `src/snapshot/limits.rs`
- Create: `src/snapshot/schema.rs`
- Create: `src/snapshot/state.rs`
- Create: `src/snapshot/codec.rs`
- Create: `src/snapshot/verified.rs`
- Create: `tests/snapshot_schema.rs`
- Create: `tests/snapshot_tampering.rs`
- Create: `tests/snapshot_state.rs`
- Create: `tests/snapshot_support.rs`
- Modify: `src/canonical/value.rs`
- Modify: `src/crypto/key_ring.rs`
- Modify: `src/identity.rs`
- Modify: `src/lib.rs`

- [x] Write failing schema tests for every required seed/instance binding: form, snapshot schema version, component name and contract, component-state schema, build, route, slot, key ID, timing, public mount parameters/state/memo, advisory generations, scope fingerprint, server instance ID, and monotonic revision.
- [x] Write failing negative tests for field tampering, component/route/slot/scope substitution, unknown/duplicate fields, expired snapshots, future issuance beyond skew, unsupported schema/build contract, forbidden private seed state, transient/secret state, and oversized/deep data.
- [x] Implement separate `SeedBodyV1` and `InstanceBodyV1` structs with `deny_unknown_fields`; do not use one bag-of-optionals schema. Store all unbounded application-shaped values as validated `CanonicalValue`/`CanonicalObject`.
- [x] Implement versioned signed-envelope encoding so signing always canonicalizes the body and verification always parses, bounds, selects purpose from the expected entry point, verifies the signature, then validates compatibility/timing/bindings.
- [x] Model public-state eligibility explicitly with schema exposure rules; forbidden state categories cannot be inserted into `SeedBodyV1` through safe constructors.
- [x] Add deterministic dehydration APIs that validate an explicit `StateSchema` before signing, plus verified hydration APIs that require `VerifiedSeedV1` or `VerifiedInstanceV1` and a caller-selected registered schema. The browser never chooses a Rust type.
- [x] Add property tests for supported state-codec round trips and explicit i64/u64/bytes tagged codecs. Verify bounded dehydration fails before any envelope exists.
- [x] Run targeted tests, full Rust unit tests, MSRV tests, and Clippy; refactor green.
- [ ] Commit: `feat: implement verified snapshot hydration`.

The signed boundary is intentionally capability-oriented:

```rust
pub fn verify_instance(
    encoded: &[u8],
    expected: &ExpectedInstance,
    keys: &SnapshotKeyRing,
    now: UnixMillis,
    limits: &SnapshotLimits,
) -> Result<VerifiedInstanceV1, LiveError>;

pub fn hydrate<T: DeserializeOwned>(
    verified: &VerifiedInstanceV1,
    schema: &StateSchema,
) -> Result<T, LiveError>;
```

## Task 5: Implement the Tier 0 instance ledger state machine

**Files:**

- Create: `src/clock.rs`
- Create: `src/ledger/mod.rs`
- Create: `src/ledger/contract.rs`
- Create: `src/ledger/memory.rs`
- Create: `src/ledger/state.rs`
- Create: `tests/ledger_claims.rs`
- Create: `tests/ledger_concurrency.rs`
- Create: `tests/ledger_expiry.rs`
- Create: `tests/ledger_support.rs`
- Modify: `src/lib.rs`

- [x] Write failing state-machine tests for atomic expected-revision claim, monotonic successor, stale base, in-progress duplicate, accepted duplicate lookup, mismatched idempotency, abandoned/expired claim consumption, instance expiry, missing ledger refresh, and bounded accepted-outcome metadata.
- [x] Write deterministic barrier-based concurrency tests that release two tasks against the same base revision and prove exactly one claim is granted. Do not use sleeps.
- [x] Define the async provider trait and typed outcomes first, then implement `MemoryInstanceLedger` with one short standard-library mutex critical section per operation and an injected `Clock`. No lock is held across unrelated awaits.
- [x] Make `ClaimToken` opaque, provider-bound, and single-use. `commit` succeeds only for the matching pending claim; `abandon` and claim-lease expiry move the instance to terminal consumed recovery. A drop or provider error does not roll the ledger backward.
- [x] Store scoped instance identity, current revision, claim/idempotency metadata, expiry, and accepted-outcome digest/category only. Metadata-only inspection and bounded-history tests prove the provider API has no component-state or response-body channel.
- [x] Run all ledger tests under Tokio with injected time, deterministic barriers, and a loom-free 128-race supplement; barrier tests remain the correctness proof.
- [ ] Commit: `feat: add Tier 0 instance revision authority`.

Provider contract outline:

```rust
#[async_trait]
pub trait LiveInstanceLedger: Send + Sync {
    async fn promote(&self, request: PromotionRecord) -> Result<PromotionOutcome, LedgerError>;
    async fn claim(&self, request: ClaimRequest) -> Result<ClaimOutcome, LedgerError>;
    async fn commit(&self, claim: ClaimToken, outcome: AcceptedOutcome) -> Result<(), LedgerError>;
    async fn abandon(&self, claim: ClaimToken) -> Result<(), LedgerError>;
}
```

## Task 6: Implement bounded public-seed promotion

**Files:**

- Create: `src/promotion/mod.rs`
- Create: `src/promotion/context.rs`
- Create: `src/promotion/error.rs`
- Create: `src/promotion/policy.rs`
- Create: `src/promotion/service.rs`
- Create: `src/random.rs`
- Create: `tests/seed_promotion.rs`
- Create: `tests/seed_promotion_limits.rs`
- Create: `tests/seed_promotion_concurrency.rs`
- Create: `tests/promotion_support.rs`
- Modify: `src/lib.rs`
- Modify: `src/snapshot/schema.rs`

- [x] Write failing tests proving integrity and binding checks precede ledger creation, a nonce has at least 128 bits, the server assigns instance identity, seed replay with a new nonce creates an independent instance, exact retry identity can recover its promotion, changed-idempotency nonce reuse is rejected, and one scoped replay cannot join/replace another scope.
- [x] Write failing tests for per-window rate, outstanding-instance, per-route/component, abandoned-retention, input-size, reservation-cardinality, and rate-bucket-cardinality limits. Use injected clocks and deterministic instance generators.
- [x] Define `TrustedPromotionContext` as adapter-supplied current route/slot/component/build/scope and verification attestations. Its name and docs state that iteration 001 does not itself implement Suprnova session, CSRF, authorization, or tenant middleware.
- [x] Implement `PromotionService`: byte bound -> verify seed and current trusted bindings -> validate typed nonce -> reserve bounded policy state -> call atomic ledger promotion -> return signed promoted instance authority. No action dispatch occurs in this iteration.
- [x] Represent `refresh_on_promote` as a typed `RefreshBeforeAction` decision carrying no claim to successful action execution. Verified advisory generations survive as memo and never become a mandatory rejection gate.
- [x] Add deterministic concurrent exact-replay promotion tests and prove pre-verification and ledger failures leave no partial authoritative instance; cancelled/failed reservations have explicit bounded recovery retention.
- [ ] Commit: `feat: add bounded public seed promotion`.

## Task 7: Implement versioned wire envelopes and response ordering

**Files:**

- Create: `src/protocol/mod.rs`
- Create: `src/protocol/error.rs`
- Create: `src/protocol/limits.rs`
- Create: `src/protocol/request.rs`
- Create: `src/protocol/response.rs`
- Create: `src/protocol/compatibility.rs`
- Create: `src/protocol/ordering.rs`
- Create: `tests/request_protocol.rs`
- Create: `tests/response_protocol.rs`
- Create: `tests/response_ordering.rs`
- Create: `tests/compatibility.rs`
- Create: `tests/protocol_support.rs`
- Modify: `src/identity.rs`
- Modify: `src/lib.rs`

- [x] Write failing request tests for explicit protocol/runtime/snapshot versions, correlation versus idempotency identity, instanced versus seed-promotion forms, base revision, bounded model proposals, bounded ordered operations, duplicate keys, unknown fields, ambiguous operation forms, and incompatible batching.
- [x] Write failing response tests for accepted/rejected/duplicate/refresh/fatal outcome distinctions, required revision/snapshot fields, explicit HTML versus no-render, redirect terminality, bounded validation/events/effects, error category/recovery agreement, and malformed partial outcomes.
- [x] Implement parsers over the bounded duplicate-aware canonical parser rather than unconstrained `serde_json::from_slice`. Validate envelope shape before snapshot verification and keep component/action/field identities unresolved pending iteration 002 registry lookup.
- [x] Implement a pure response-application planner whose ordered semantic steps cover terminal redirect; morph-before-commit; no-render validation-before-commit; rejection retention; explicit refresh/fatal recovery; and post-acceptance morph failure without request replay.
- [x] Implement exact compatibility windows for protocol v1/snapshot v1/runtime contract v1. Unknown breaking versions produce one document-refresh decision; optional extensions are accepted only through a namespaced bounded map.
- [x] Add A8/16 control-overhead assertions capped at 1 KiB and signed snapshot framework-overhead assertions capped at 768 bytes.
- [x] Run targeted tests, full Rust tests, MSRV tests, and warning-free Clippy; refactor green.
- [ ] Commit: `feat: define Live v1 wire protocol contracts`.

Ordering model example:

```rust
pub enum ApplicationStep {
    Navigate,
    PreflightMorph,
    Morph,
    ValidateNoRender,
    CommitSnapshotAndRevision,
    ReconcileModelsAndValidation,
    RestoreFocus,
    DispatchEvents,
    RunRegisteredEffects,
    SettleFeedback,
    RequestFreshRenderWithoutReplay,
}
```

## Task 8: Create shared golden fixtures and TypeScript conformance

**Files:**

- Create: `fixtures/v1/canonical-success.json`
- Create: `fixtures/v1/canonical-failure.json`
- Create: `fixtures/v1/snapshot-success.json`
- Create: `fixtures/v1/snapshot-failure.json`
- Create: `fixtures/v1/protocol-success.json`
- Create: `fixtures/v1/protocol-failure.json`
- Create: `fixtures/v1/response-ordering.json`
- Create: `fixtures/v1/compatibility.json`
- Create: `fixtures/v1/manifest.sha256`
- Create: `src/conformance.rs`
- Create: `tests/golden_fixtures.rs`
- Create: `browser/src/canonical.ts`
- Create: `browser/src/schema.ts`
- Create: `browser/src/crypto.ts`
- Create: `browser/src/protocol.ts`
- Create: `browser/src/ordering.ts`
- Create: `browser/src/conformance.ts`
- Create: `browser/tests/golden-fixtures.test.ts`
- Create: `browser/scripts/check-budget.mjs`
- Modify: `browser/src/index.ts`
- Modify: `browser/tsconfig.build.json`
- Modify: `src/lib.rs`

- [x] Add a deliberately minimal fixture loader and write failing Rust and TypeScript tests that both load it by repository-relative path. Observe both fail before implementing the fixture codecs.
- [x] Expand the reviewed v1 corpus across canonical success/failure, seed/instance integrity and expiry, request/response acceptance and rejection, ordering, and compatibility classes. Fixed root keys are labelled public non-production test vectors; expected failures are stable closed codes.
- [x] Implement strict TypeScript schema guards with own-property checks, duplicate-aware input parsing, explicit depth/count/byte/string limits, decimal-string handling, base64url identities, and no `any` or unsafe external-boundary assertions.
- [x] Implement RFC 8785-compatible canonicalization in TypeScript using UTF-16 property ordering and ECMAScript number serialization, then purpose-separated HKDF/HMAC verification through Web Crypto against Rust-produced signed fixture bytes.
- [x] Implement the TypeScript response-application planner as a pure semantic model with no DOM access or iteration 003 runtime claim.
- [x] Make Rust and TypeScript enumerate every reviewed fixture case and reject unknown case kinds or missing expectations. Both compute and compare the same exact ordered fixture-manifest SHA-256.
- [x] Implement `npm run budget` for the 1 KiB control and 768-byte snapshot overhead caps. Run format, lint, typecheck, tests, build, and budget.
- [x] Commit: `test: add cross-language Live v1 conformance`.

## Task 9: Add hostile-input, property, fuzz, and telemetry-bound coverage

**Files:**

- Create: `src/telemetry.rs`
- Create: `tests/security_boundaries.rs`
- Create: `tests/parser_properties.rs`
- Create: `tests/fuzz_regressions.rs`
- Create: `fuzz/Cargo.toml`
- Create: `fuzz/fuzz_targets/canonical_input.rs`
- Create: `fuzz/fuzz_targets/signed_snapshot.rs`
- Create: `fuzz/fuzz_targets/update_request.rs`
- Create: `fuzz/fuzz_targets/update_response.rs`
- Create: `fuzz/corpus/README.md`

- [x] Write failing redaction and bounded-cardinality tests before telemetry types. Prove labels derive only from closed enums and bounded digests, never raw component/action/route/key IDs or payloads.
- [x] Add the complete negative matrix: tampering, malformed canonical data, wrong key/purpose, expired/retired keys, cross-binding substitution, nonce reuse/replay/exhaustion, stale/duplicate/consumed revisions, oversized/deep input, malformed error output, and no panic from external bytes.
- [x] Add Proptest strategies for valid canonical trees within limits, schema-evolution extensions, malformed envelopes, and random mutation of signed data. Persist every discovered regression as a small corpus fixture.
- [x] Add one fuzz target for each external parser/verifier. Each target installs fixed test-only keys and strict tiny limits; it must return classified errors without panicking. Build with `rtk cargo +nightly fuzz build` and run a deterministic bounded smoke campaign for every target.
- [x] Verify production sources contain no `unsafe`, `todo!`, `unimplemented!`, or hostile-input `unwrap`/`expect`; use structural/literal scans only after tests and Clippy.
- [x] Commit: `test: harden Live v1 external boundaries`.

## Task 10: Implement the A8/16 performance and byte-budget harness

**Files:**

- Create: `benches/snapshot_budget.rs`
- Create: `benchmarks/s1-environment.schema.json`
- Create: `benchmarks/snapshot-budget-v1.json`
- Create: `scripts/run-snapshot-budget.sh`
- Create: `docs/implementation/benchmarking.md`

- [x] Write a failing fixture-budget test first with deliberately over-budget metadata; observe the control/snapshot assertion fail, then switch to the real named fixture and optimize only if needed.
- [x] Add a `harness = false` release benchmark that performs verify -> hydrate -> deterministic dehydrate -> canonicalize -> sign for 8 KiB state, warms up, records at least 30 batch samples, computes p50/p95, and fails above 500 microseconds p95.
- [x] Record CPU model, architecture, selected eight-CPU affinity, memory, kernel, governor, rustc, profile, warmup/sample counts, fixture SHA-256, p50, and p95 in a versioned JSON result. The runner must distinguish validated S1 evidence from local exploratory measurements rather than labelling arbitrary hardware S1.
- [x] Add correctness assertions around the benchmark result so invalid signatures, weakened limits, or skipped stages cannot create a passing fast path.
- [ ] Run the benchmark through the script. If the host cannot prove the S1 environment, retain the honest local measurement and run the release-blocking measurement on a qualifying runner before completing the iteration; do not fabricate S1 metadata.
- Local evidence on 2026-08-21 is retained as `local_exploratory`: p95 69.043 microseconds, eight selected CPUs, `powersave` governor, and no dedicated-vCPU attestation. The explicit `SUPRNOVA_LIVE_REQUIRE_S1=1` run failed closed as designed; qualifying S1 evidence remains release-blocking.
- [x] Commit: `perf: add Live snapshot processing budget`.

## Task 11: Document the implemented v1 contract and build the unattended gate

**Files:**

- Create: `README.md`
- Create: `docs/implementation/snapshot-v1.md`
- Create: `docs/implementation/protocol-v1.md`
- Create: `docs/implementation/threat-model-v1.md`
- Create: `docs/implementation/fixtures.md`
- Create: `scripts/gate.sh`
- Modify: `.gitignore`
- Modify: `docs/specs/suprnova-live/05-snapshots-and-hydration.md`
- Modify: `docs/specs/suprnova-live/06-wire-protocol-and-transport.md`
- Modify: `docs/specs/suprnova-live/07-security-and-trust-boundaries.md`
- Modify: `docs/specs/suprnova-live/19-developer-tooling-and-testing.md`
- Modify: `docs/specs/suprnova-live/glossary.md`
- Modify: `docs/specs/suprnova-live.zip`

- [x] Write a failing shell-gate contract test before `scripts/gate.sh`: it must reject blanket `-D warnings`, omitted Rust/TypeScript fixture parity, omitted security/fuzz build, or omitted budget command.
- [x] Implement `scripts/gate.sh` as unattended fail-fast orchestration of the exact iteration commands. It must use `CARGO_INCREMENTAL=0`, never blanket-deny warnings, build fuzz targets with the pinned nightly toolchain, and print each phase without leaking environment secrets.
- [x] Document every in-scope boundary, threat assumption, provider guarantee, failure/recovery outcome, fixture format, benchmark reproduction command, MSRV, feature declaration, and internal/future-facade distinction. Explicitly label session/CSRF/auth/tenant/HTTP/DOM work as iteration 002/003 rather than implemented.
- [x] Record exact schema/profile decisions in specs 05-07 and shared-fixture/budget harness placement in spec 19. Add only genuinely project-specific terms to the glossary. Keep decision logs newest-first and update `Last revised` consistently.
- [x] Narrow `.gitignore` so normative specs and implementation docs are tracked intentionally while the large `reference/`, targets, `browser/node_modules`, browser build output, fuzz artifacts, and local benchmark scratch stay ignored. Preserve unrelated local `.agents`, `.claude`, `.codex`, and `skills-lock.json` files.
- [x] Regenerate the optional Fable ZIP because it exists, then run the spec checker and ZIP equality check.
- [x] Commit: `docs: record Live v1 trusted interaction contracts`.

## Task 12: Run the complete iteration gate and final self-audit

**Files:**

- Review: every tracked iteration 001 file
- Modify only defects proven by checks/review

- [ ] Run `rtk node scripts/check-specs.mjs`.
- [ ] Run `rtk git diff --check`.
- [ ] Run `rtk env CARGO_INCREMENTAL=0 cargo fmt --all --check`.
- [ ] Run `rtk env CARGO_INCREMENTAL=0 cargo clippy --all-targets --all-features` and review every warning without `-D warnings`.
- [ ] Run `rtk env CARGO_INCREMENTAL=0 cargo test --all-targets --all-features --no-fail-fast`.
- [ ] Run `rtk env CARGO_INCREMENTAL=0 cargo test --doc --all-features`.
- [ ] Run `(cd browser && rtk npm ci)`.
- [ ] Run `(cd browser && rtk npm run format:check)`.
- [ ] Run `(cd browser && rtk npm run lint)`.
- [ ] Run `(cd browser && rtk npm run typecheck)`.
- [ ] Run `(cd browser && rtk npm test)`.
- [ ] Run `(cd browser && rtk npm run build)`.
- [ ] Run `(cd browser && rtk npm run budget)`.
- [ ] Run `rtk env CARGO_INCREMENTAL=0 scripts/gate.sh`.
- [ ] Run the MSRV matrix with `rtk env CARGO_INCREMENTAL=0 cargo +1.91.1 test --all-targets --all-features --no-fail-fast` and `rtk env CARGO_INCREMENTAL=0 cargo +1.91.1 clippy --all-targets --all-features`.
- [ ] Run `rtk cargo +nightly fuzz build` plus bounded smoke campaigns for all four targets.
- [ ] Inspect the complete diff and tracked-file inventory. Search for placeholders, unsafe, blanket warning denial, unbounded external deserialization, secret-bearing formatting, accidental iteration 002-007 claims, and edits outside this repository.
- [ ] Verify `/home/shawn/workspace2/suprnova` and `/home/shawn/workspace2/suprnova-magnetar` statuses are unchanged from their read-only baselines.
- [ ] Re-run any check affected by a remediation. Do not mark the task complete until the exact full gate and required S1 evidence pass.
- [ ] Commit the final verified state locally with no push: `feat: complete Suprnova Live iteration 001`.

## Plan self-review checklist

- [x] Every item in iteration 001 `In` and Definition of Done maps to a task and executable verification above.
- [x] Every iteration 001 `Out` capability remains explicitly out; no task adds component macros/actions, the endpoint, session/auth adapters, DOM runtime, uploads, RenderCache, component catalog, or Suprnova integration.
- [x] Every hostile browser value crosses a byte/count/depth bound and typed validator before expensive work.
- [x] Signature integrity, request authenticity, current authorization, scope binding, domain freshness, and secrecy are never conflated.
- [x] State publication and browser application order cannot commit a snapshot before a successful morph/no-render validation.
- [x] The ledger does not become server-resident component state and does not promise exactly-once method invocation/effects.
- [x] Rust/TypeScript fixture parity is one checked-data set, not manually duplicated expectations.
- [x] The standalone development boundary is preserved and no source dependency points at active Suprnova.
- [x] No blanket `-D warnings`, production `unsafe`, placeholder adapter, sleep-based concurrency proof, or fabricated benchmark environment is permitted.
