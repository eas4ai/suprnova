# Suprnova Live -- 08 File Uploads

Status: Normative design specification
Last revised: 2026-08-24

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
- The optional upload artifact proposes only the opaque handle or `null` to the
  declared upload field through a core-validated typed capability; it cannot
  write another model field or carry the transfer grant into an action/model
  envelope.
- File controls remain subject to native browser security restrictions.

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

Acceptance criteria:
- Cancellation stops future chunks and invalidates or marks the temporary
  reference.
- Cleanup is idempotent and safe under concurrent finalize/cancel races.
- Background cleanup has observable age, volume, failure, and retry metrics.
- A browser disconnect does not leave permanent unowned files.
- Removal updates component state and validation without forging native file
  input values. The runtime may assign only `input.value = ""` to clear a
  retired native selection; assigning a non-empty value or `input.files` is
  forbidden.
- Cross-reload resume requires an explicit authenticated application route that
  lives outside the reserved `/__live/` namespace, reauthorizes the opaque
  handle, and issues a new transfer grant; no default localStorage,
  sessionStorage, or IndexedDB persistence is permitted.

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
- Progress is announced without excessive live-region noise.
- Cancel, remove, and retry actions have accessible names and focus behavior.
- Active `File` objects, transfer grants, and progress tasks survive only while
  their current-document keyed owner and policy remain valid.
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
