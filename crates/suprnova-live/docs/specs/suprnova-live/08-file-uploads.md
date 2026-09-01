# Suprnova Live -- 08 File Uploads

Status: Normative design specification
Last revised: 2026-09-01

## Scope

This domain owns browser-to-server file selection, transfer, temporary upload
identity, progress, validation, cancellation, retry/resume, cleanup, and final
action attachment. It depends on state binding, wire transport, security, and
Suprnova storage. It feeds actions and form components without placing file
bytes, transfer authority, or trusted file metadata inside component snapshots.
The executor-neutral engine delegates raw quarantine byte I/O through a host
capability. Its reference provider retains path policy, hashing, and state while
the Rust test-support host supplies the Tokio filesystem adapter;
provider-neutral direct storage preserves the same authority and lifecycle
contract.

## Capabilities

### File selection and temporary identity

A model-bound file control shall create a pending upload represented to the
component by an opaque, principal-bound upload handle. The handle identifies
temporary state but grants no transfer or finalization authority. A separate
short-lived transfer grant authorizes only bounded control/data operations and
never enters component state or rendered markup. Browser file paths, names,
MIME claims, and snapshot fields shall not become trusted storage identity.

Acceptance criteria:

- Single and multiple file selection declare count, size, and accepted-type
  constraints.
- File bytes never enter the JSON control envelope or signed component state.
- Temporary references are unguessable, expiring, scoped to the correct
  principal/session, component, field, and tenant.
- Handle use is reauthorized at every control and finalization boundary; handle
  possession alone grants nothing.
- Transfer grants are scope- and operation-limited secrets and never appear in
  snapshots, HTML, URLs, history, action/model envelopes, logs, traces,
  diagnostics, or inspection output.
- Selecting a replacement file retires or preserves the previous temporary
  upload according to explicit form policy.
- The optional upload artifact proposes only `null`, one opaque handle, or a
  bounded ordered handle list for a multiple-file control to the declared
  upload field through a core-validated typed capability; it cannot write
  another model field or carry the transfer grant into an action/model envelope.
- File controls remain subject to native browser security restrictions.
- The declared native file input's current `FileList` is selection input only
  when that input is the change-event target. Event `isTrusted` is not promoted
  into file or server-validation authority, and an unrelated bubbled change
  event cannot start a transfer.

UX flow:

1. Application user selects permitted files -> the form creates pending uploads
   and exposes per-file progress.
2. Selection violates immediate client-known constraints -> the control reports
   the issue without claiming server validation.

### Bounded transfer and progress

Uploads shall stream or chunk file data through a bounded transport that reports
progress and supports backpressure without buffering entire large files in
browser runtime or server memory.

Acceptance criteria:

- Transfer endpoints require current request authenticity and temporary-upload
  authorization.
- Configurable file, request, chunk, concurrency, and total-pending limits are
  enforced server-side.
- Progress distinguishes queued, transferring, verifying, ready, finalizing,
  finalized, interrupted, failed, canceled, and expired states where material.
- Authoritative state progresses conditionally by upload revision through
  created, queued, transferring, verifying, ready, finalizing, and finalized,
  with rejected, canceled, expired, and failed terminal dispositions.
- Backpressure prevents aggregate uploads from exhausting process memory or
  storage descriptors.
- Checksums or equivalent integrity checks detect corrupt/incomplete transfer
  when resume or chunking is supported.
- Reverse-proxy/file transfer is the daemon-free reference provider.
- The engine defines an asynchronous `QuarantineStore` capability and performs
  no blocking filesystem calls. The Tokio-backed reference file implementation
  and its HTTP body adaptation live in the test-support host.
- Direct-to-storage is a provider-neutral capability with conformance for
  constrained instructions, integrity, completion, verification, cancellation,
  expiry, and cleanup; every provider preserves the same authority model.
- Count, aggregate bytes, creation rate, temporary storage, verification time,
  in-flight work, retry, and retention are bounded in addition to per-file and
  per-chunk limits.

The production optional artifact owns one upload manager per document and one
current-document transfer owner per selected file. It uses the shared bounded
owner for queueing, permits, lifecycle, and disposal; defaults to four active
files and 256 KiB chunks; retains at most one uncertain chunk in JavaScript and
never more than two chunk buffers per active transfer; and computes chunk plus
whole-file SHA-256 incrementally without reading the whole file into framework
memory. Public configuration cannot exceed 16 active transfers, 64 files, 4 MiB
chunks, or 4 MiB of manager queue accounting. Transport, connectivity, retry
randomness, and the optional application reacquisition port are injected before
runtime boot through `configureUploads`; `resumeUpload` is the supported
document/island-scoped explicit reacquisition entry rather than a second feature
registration. The
reference fetch adapter uses the fixed reserved `/__live/v1/upload` endpoint and
places a transfer grant only in the `Authorization` header, never in the URL,
history, diagnostics, or model proposal. It reads upload control responses
through a 16 KiB bounded stream rather than an unbounded `response.json()`.
Every typed response is checked against its operation and cannot regress the
expected revision. Server-terminal failure, cancellation, expiry, or
finalization clears the proposal, file, grant, handle, and uncertain bytes.
Completion retains one idempotency key until acknowledgment; a lost completion
response first reconciles through read-only status and resends only when the
authoritative state remains transferable.

UX flow:

1. Accepted file enters the upload queue -> progress advances without blocking
   unrelated island interactions beyond declared scheduling.
2. Transfer is interrupted in the current document -> the file becomes
   retryable/resumable or failed according to the backend contract; no
   completion is reported prematurely.

### Server validation and quarantine

The server shall validate completed temporary files using authoritative bytes
and policy, including size, content inspection where configured, extension/name
sanitization, and application validation. Untrusted files shall remain isolated
from public or durable storage until accepted.

Acceptance criteria:

- Server validation does not rely solely on browser MIME or extension claims.
- Accepted upload types are digest-significant canonical MIME/extension
  contracts. The bounded engine authoritatively classifies PNG/JPEG/GIF/WebP;
  other formats such as PDF require an explicit trusted application classifier
  over quarantined content before a browser claim may agree with the result.
- Scanner and application-classifier ports receive only immutable inspection
  facts plus a provider-neutral, read-only, chunk-bounded content view and an
  absolute host-enforced deadline; they receive no transfer or storage-mutation
  capability.
- Original names are treated as display metadata, normalized, bounded, and
  prevented from controlling storage paths.
- Malware/content scanning hooks can quarantine pending files without blocking
  the main process indefinitely.
- Scanning defines timeout, unavailable, rejection, retry, and cancellation
  disposition rather than treating silence as success.
- Validation errors associate with the correct form field and file.
- Rejected or quarantined files cannot be finalized or served publicly.
- Image or media metadata parsing is bounded against decompression and parser
  abuse.
- The reference image-dimension probe uses exact `imagesize` 0.15.0 with default
  features disabled and only PNG/JPEG/GIF/WebP enabled. It reads a capped byte
  prefix, fails closed when the necessary header exceeds that cap, and is covered
  by a dedicated hostile-input corpus and fuzz target. Full media decoding is an
  application validation capability.

UX flow:

1. Transfer completes -> server verification begins and the UI distinguishes it
   from final acceptance.
2. Validation rejects a file -> the field explains the safe actionable reason
   and permits removal or replacement.

### Finalization with an action

A temporary upload shall become durable only through an authorized application
action that consumes its reference and commits the intended domain operation.
Finalization shall be idempotent and coordinate storage with database state.

Acceptance criteria:

- The action reauthorizes ownership and revalidates the temporary reference.
- A reference cannot be consumed by another principal, field, tenant, or
  component.
- Durable naming and visibility are generated by trusted application/storage
  policy.
- Database and storage partial failure has a documented compensation or retry
  path.
- Provider preparation, durable commit, compensation, retry, and reconciliation
  are explicit; Live does not claim a distributed transaction across storage
  and the application database.
- Immutable validation evidence binds authoritative inspection facts, complete
  scope, policy digest, and exact Ready revision. A finalizer token is host-only
  idempotency identity and never browser or public-storage authority.
- Repeated finalization cannot duplicate a file or domain record.
- A completed temporary upload may expire if never finalized.

UX flow:

1. Application user submits the form after uploads complete -> the action
   validates and atomically or compensatably finalizes referenced files.
2. Finalization fails -> existing pending state and retry guidance reflect
   whether any durable work committed.

### Cancellation, removal, and cleanup

Application users shall be able to cancel pending transfers and remove
temporary uploads where policy permits. Server cleanup shall reclaim abandoned,
expired, rejected, and canceled data even when the browser disappears.

Cleanup authority is ledger-owned. One bounded claim operation first advances
an expired `Created`, `Queued`, `Transferring`, `Verifying`, or `Ready` record to
`Expired` under its exact current revision, or selects an already `Rejected`,
`Canceled`, `Expired`, or `Failed` record, then installs a short lease bound to
the resulting revision. `Finalizing` and `Finalized` records are never eligible.
Provider deletion and validation-evidence removal run outside the ledger lock
and are idempotent; terminalization succeeds only while the exact lease and
revision remain current, then removes the reclaimed temporary authority record.
A duplicate completion for an absent record is stale rather than destructive.
A stale completion is fenced and later reconciliation repeats deletion safely.

Each run has independent nonzero item and aggregate retained-byte limits, uses
the shared resource queue, cancellation flag, and permit pool, and requires no
browser callback. Due-work selection is itself ordered and bounded; an adapter
must not scan or allocate the entire ledger to return one batch. Failed
reclamation uses capped exponential backoff. Crossing the finite orphan
threshold marks the record for operations without abandoning it: capped
reconciliation continues until physical cleanup is confirmed.

Acceptance criteria:

- Cancellation stops future chunks and invalidates or marks the temporary
  reference.
- Cleanup is idempotent and safe under concurrent finalize/cancel races.
- Cleanup claims and completions are revision- and lease-fenced; a worker never
  holds the ledger lock across provider I/O.
- Claim selection work is bounded independently of total ledger size, and
  successful completion removes temporary authority without retaining an
  unbounded tombstone set.
- Background cleanup has closed age, volume, outcome, retry, and orphan metrics
  without upload, filename, path, scope, principal, grant, or raw-error labels.
- A browser disconnect does not leave permanent unowned files.
- Removal updates component state and validation without forging native file
  input values. The runtime may assign only `input.value = ""` to clear a
  retired native selection; assigning a non-empty value or `input.files` is
  forbidden.
- Cross-reload resume requires an explicit authenticated application route that
  lives outside the reserved `/__live/` namespace, reauthorizes the opaque
  handle, and issues a new transfer grant; no default localStorage,
  sessionStorage, or IndexedDB persistence is permitted.
- Reacquisition returns the exact authoritative uploaded-byte offset and next
  chunk index; neither is reconstructed from the current deployment's chunk
  size. The
  browser verifies the user-held file identity, incrementally rehashes the
  already accepted prefix under the same chunk bound, reads authoritative
  status, and only then resumes without issuing a second create request.

UX flow:

1. Application user cancels or removes a pending file -> its progress stops and
   the field returns to the appropriate empty or remaining-files state.
2. Browser vanishes -> expiry and cleanup reclaim the temporary data without a
   client callback.

### Morph and accessibility integration

Upload controls shall survive compatible island morphs without violating native
file-input security. Progress and validation shall be operable and perceivable
through keyboard and assistive technology.

Acceptance criteria:

- Morphing never attempts to programmatically restore an arbitrary local file
  path.
- Active file inputs and progress roots use explicit keys/preservation rules.
- An active field survives only when its native input, progress root, and
  declared controls retain the same explicit keys and DOM identities. Removal,
  forced replacement, or rekeying retires the prior field once and requires a
  new selection.
- Progress exposes bounded aggregate bytes and percent, never reports success
  before the authoritative ready state, and announces state changes immediately
  while throttling numeric live-region updates.
- Cancel, remove, and retry actions have accessible names and focus behavior.
- Active `File` objects, transfer grants, and progress tasks survive only while
  their current-document keyed owner and policy remain valid.
- bfcache suspension aborts active browser work into an explicit interrupted
  state while retaining current-document retry authority; navigation or owner
  retirement clears the native input only with `input.value = ""` and releases
  secrets and resources once.
- Navigation or intentional removal warns about active uploads when the
  application policy requires it.

UX flow:

1. Unrelated action morphs the form -> active permitted uploads and progress
   continue under stable identity.
2. Upload boundary must be replaced -> the UI communicates cancellation or
   required reselection instead of silently losing the file.

## Acceptance criteria

- File bytes and trusted metadata never enter snapshots or JSON action
  envelopes.
- Transfer is bounded, observable, authenticated, and backpressured.
- Temporary files remain quarantined until server validation and authorized
  finalization.
- Cancellation, retry/resume, finalization, and cleanup are race-safe and
  idempotent.
- Upload UI preserves browser security and accessibility across morphs.

## Decisions and revisions

- 2026-09-01 -- Integrated upload control and reverse-proxy data transfer through
  the fixed versioned `/__live/v1/upload` route while keeping reacquisition an
  explicitly registered authenticated application route outside `/__live/`.
  Current middleware facts and application authorization gate every operation;
  bounded per-handle serialization covers transfer, cancellation, cleanup, and
  finalization; body-memory permits are acquired before buffering; and configured
  direct providers retain the same signed authority, revision, validation,
  cancellation, expiry, and cleanup state machine as the daemon-free provider.
- 2026-08-25 -- Closed the upload security gate with bounded arbitrary-byte
  protocol, lifecycle-sequence, and capped media-header fuzz targets. Browser
  conformance now keeps a live grant sentinel absent from rendered HTML,
  URL/history, storage, console, and request traces; the static test host emits
  no literal grant, defines no quarantine-serving route, and upload resume still
  requires current memory or explicit authenticated application reacquisition.
- 2026-08-25 -- Implemented truthful accessible upload projection and strict
  keyed continuity. Native file selection is read only from the declared input
  target; same-key/same-node input, progress, and control identity survives a
  morph, while removal/replacement/rekey retires once, clears only with the
  empty-string native assignment, and never assigns `files` or a path. bfcache
  suspension becomes explicit interruption and navigation retires authority.
- 2026-08-25 -- Bound resumed transfer to both the authoritative byte offset and
  next chunk index, including status reconciliation, so deployment-time chunk
  configuration changes cannot corrupt provider sequencing. Per-island/field
  generations discard late grants after newer selection, removal, retirement,
  or document disposal.
- 2026-08-25 -- Hardened the production browser artifact with operation-specific
  response states, non-regressing revisions, terminal authority disposal,
  stable completion idempotency plus status reconciliation, upload-specific
  hard ceilings, and real offset-bearing reacquisition. The automatically
  registered singleton now exposes pre-boot `configureUploads` and scoped
  `resumeUpload` instead of advertising an unusable replacement factory.
- 2026-08-25 -- Implemented the current-document browser owner on the shared
  bounded-resource foundation with injected transport/connectivity/randomness,
  256 KiB default chunks, incremental SHA-256, uncertain-chunk idempotent retry,
  strict native-input clearing, and immediate secret disposal. Multiple-file
  fields use a bounded ordered handle list. Core proposal authority validates
  declaration, UUID grammar, island/field scope, and retirement before the
  existing model batch may observe the value.
- 2026-08-25 -- Defined cleanup as a bounded ledger-owned claim protocol. Active
  expiry is atomic with the claim, `Finalizing`/`Finalized` are ineligible,
  physical deletion is idempotent and outside the ledger lock, and completion is
  revision/lease-fenced. Due selection cannot scan the entire ledger, successful
  completion removes temporary authority without permanent tombstones, and
  capped retries continue after an orphan threshold. Cleanup uses shared
  resource primitives and closed redacted metrics without browser cooperation.
- 2026-08-24 -- Made accepted-type metadata extensible beyond the four bounded
  image probes: custom types require a trusted application classifier over
  authoritative quarantined content, while browser MIME and filename claims
  remain non-authoritative. Scanner/classifier ports receive a bounded read-only
  content view and deadline rather than storage mutation authority. Bound
  validation evidence and host-only finalize tokens to exact scope, policy, and
  lifecycle revision.
- 2026-08-24 -- Kept the engine executor-neutral with a host-owned asynchronous
  `QuarantineStore`; the Tokio file provider and dynamic reference HTTP host live
  in test support. Added the core-validated upload-handle proposal capability,
  bounded `imagesize` 0.15.0 probing with fuzz coverage, empty-string-only native
  input clearing, and application-owned reacquisition routes outside `/__live/`.
- 2026-08-23 -- Locked Iteration 004 upload architecture around a non-authority
  opaque handle plus a separate secret transfer grant, a revisioned temporary
  lifecycle, a daemon-free reverse-proxy/file reference provider, and one
  provider-neutral direct-storage conformance contract. Guaranteed resume is
  current-document only; cross-reload reacquisition is an explicit authenticated
  application path rather than default browser persistence.
- 2026-08-21 -- File uploads receive a dedicated domain because their transport,
  temporary lifecycle, and security cannot be reduced to ordinary model binding.
- 2026-08-21 -- Snapshots carry only opaque temporary references, never file
  bytes or browser-asserted trusted metadata.
