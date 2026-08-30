# Uploads

Iteration 004 implements the standalone upload engine, upload protocol v1, and
the optional `uploads@1` browser artifact. This is engine and conformance
machinery for the future internal `suprnova::live` facade. It does not claim
that Suprnova routes, sessions, storage, scanners, or application macros are
already integrated.

Suprnova itself can render pages and process ordinary forms without JavaScript.
Live initial content is server-rendered HTML. `live:upload`, Live actions, model
synchronization, progress, retry, and current-document resume require the Live
browser runtime; Live does not synthesize a second no-JavaScript path for those
operations. An application that needs one supplies an ordinary Suprnova form.

The checked Askama grammar is ordinary external HTML with declarative
attributes. A current upload field can be authored as follows:

```html
<label for="avatar">Avatar</label>
<input
  id="avatar"
  type="file"
  accept="image/png,image/jpeg"
  live:upload="avatar"
  data-suprnova-live-key="avatar-input"
  aria-describedby="avatar-status avatar-error"
/>
<div
  id="avatar-status"
  live:progress="avatar"
  data-suprnova-live-key="avatar-progress"
  role="progressbar"
  aria-label="Avatar upload progress"
  aria-errormessage="avatar-error"
  aria-valuemin="0"
  aria-valuemax="100"
  aria-valuenow="0"
></div>
<p id="avatar-error" hidden>Avatar upload failed.</p>
<button type="button" live:upload.cancel="avatar">Cancel upload</button>
<button type="button" live:upload.retry="avatar">Retry upload</button>
<button type="button" live:upload.remove="avatar">Remove upload</button>
```

The standalone engine currently expresses policy through checked metadata. The
future Suprnova integration may wrap this in a macro or attribute, but no such
application-facing syntax is claimed here. The lower-level shipped form is:

```rust
let avatar = FieldMetadata::new(
    ModelField::parse("avatar")?,
    FieldCategory::Model,
    StateCodec::Json,
    true,
)
.with_model_binding(ModelCodec::String, BindingTiming::Change)?
.with_upload_policy(UploadFieldPolicy::new(
    4,
    4 * 1024 * 1024,
    UploadReplacementPolicy::RetirePrevious,
    vec![UploadMediaType::Png, UploadMediaType::Jpeg],
    None,
    UploadScanPolicy::Disabled,
    ActionName::parse("save")?,
)?)?;
```

The checker binds `live:upload="avatar"`, its progress and control roles, the
model field, accepted types, replacement policy, scan policy, and registered
finalize action into one component contract. A template cannot add upload
authority that metadata did not declare.

## Handle and grant

An `UploadHandle` is a canonical UUIDv4 or UUIDv7 temporary identity. It is safe
to dehydrate as component state, render as an opaque identifier when needed,
and propose through the typed `UploadIslandPort::proposeUploadHandle` boundary.
It is not proof that the bearer may read, mutate, or finalize the upload.

A `TransferGrant` is secret bearer authority. Its signed scope binds the handle,
component, field, principal, session, tenant, host scope, protocol version, and
expiry. The browser retains it only inside the upload manager and sends it only
as the upload authorization credential. A grant is never persisted, rendered,
or logged. It is never component state, a URL, local or persistent storage, or a
diagnostic field. Debug representations and public observers expose redacted
values or counts, never the token.

Creation and every control operation reauthorize current trusted request
context. The reference limit profile admits at most 16 files per field, 128
pending uploads per authority scope, 64 creations per 60-second window, 64 MiB
per file, 256 MiB in aggregate, and eight concurrent transfers. These are
bounded reference defaults selected through `UploadLimitConfig`; applications
may choose stricter values but cannot exceed engine ceilings. Revision CAS and
bounded idempotency history make exact retries deterministic without claiming
exactly-once external behavior.

The closed `UploadErrorKind` taxonomy separates invalid or expired grants,
scope mismatch, current authorization failure, rate/file/pending limits,
revision conflicts, unavailable ledger/provider/scanner/finalizer work,
integrity and media failures, cancellation, expiration, cleanup timeout, and
resource exhaustion. Browser recovery follows the authoritative state:
retryable interruption can retry or reacquire, an upload conflict/status loss
refreshes status, expired or denied authority requires a new authorized upload,
and terminal rejected/canceled/expired/failed state does not resume silently.

## Provider modes

`UploadProvider` is provider-neutral. `TransferPlan` selects exactly one of two
modes while keeping the same handle, grant, lifecycle, revisions, validation,
finalization, cancellation, and cleanup semantics.

In reverse-proxy mode the authenticated Live upload route receives a bounded
streaming request body. `ChunkBody` and `QuarantineStore` move bounded segments;
the engine checks declared part, byte limit, checksum, current grant, revision,
retry identity, and cancellation before committing the chunk outcome. The file
reference provider uses asynchronous Tokio file I/O and server-generated opaque
quarantine object names. A client filename remains untrusted display metadata
and never becomes a filesystem path.

In direct-provider mode the server emits a short-lived
`DirectTransferInstruction`. It fixes the trusted provider origin, HTTP method,
required header set, part range, provider reference, exact byte ceiling, and
exclusive expiry. The browser may send those bytes directly, but it reports the
part result back through authenticated Live control before the upload advances.
The provider reference is not application authority, and a browser cannot
choose another origin, method, header, range, or size. The shipped direct
provider bridge and `DirectProviderConformanceAdapter` prove this contract
against the reference host; they are not a vendor SDK or production vendor
integration.

## Quarantine and scanning

All accepted bytes remain temporary quarantine data until explicit application
finalization. Completion moves the lifecycle from `Transferring` to
`Verifying`; it does not make the file durable or trusted.

`UploadValidationService` reads authoritative quarantine bytes and verifies the
actual byte count and whole-file SHA-256. Accepted type policy uses detected
content, not the browser filename or declared MIME type. The bounded media
parsers recognize PNG, JPEG, GIF, and WebP headers and prove configured width,
height, and pixel limits. A truncated, malformed, or unproved recognized header
fails as `MediaHeaderUnproved` rather than being guessed safe.

`UploadScanPolicy` is either disabled or required with explicit timeout and
unavailable policies. Required scans run through the host-owned asynchronous
`UploadScanner`; timeout and unavailable outcomes follow the field's
`ScanFailurePolicy` instead of defaulting to acceptance. Application-specific
validation runs only after built-in authoritative inspection and under its own
deadline. Rejection reasons are closed and user-correctable: size, integrity,
type, header, dimensions, pixels, scan rejection/timeout/unavailability, or
application rejection.

Accepted immutable `ValidatedUpload` evidence binds the full authority scope,
policy digest, inspection facts, and exact Ready revision. The validation store
persists that evidence before the `Ready` transition. Missing, stale, or
conflicting evidence fails closed.

## Finalization and compensation

`Ready` means validation succeeded and the temporary object may be offered to
the registered finalize action. It does not mean durable storage or an
application database update has committed. Finalization accepts only a current
authorized action whose component and action identity match the field policy,
plus the exact Ready revision and an idempotency key.

`UploadFinalizationService` coordinates the shipped `UploadFinalizer` contract:

1. `prepare` idempotently binds a host operation token to the validated request.
2. `commit` idempotently produces one `DurableUpload` application/storage
   identity.
3. `compensate` cleans an invalid prepared result or prepared work after commit
   failure.
4. `reconcile` discovers a durable outcome when an external commit may have
   succeeded but the upload ledger did not record it.

Only a coherent Ready/Finalizing/Finalized revision may enter that sequence.
The begin transition commits `Finalizing` before the finalizer port runs. A
prepare error propagates while the upload remains `Finalizing`; it does not call
`compensate`. That state permits a later retry to reconcile first and then
prepare again when no durable outcome exists.

Compensation is attempted only for an invalid prepared result or a commit
failure. If that compensation cannot be confirmed, the engine returns
`CompensationFailed` and the host must reconcile the external operation before
deciding retry or cleanup. A durable result that does not match the request, or
a durable commit whose ledger transition is lost, returns
`ReconciliationRequired`. Exact replays return
`FinalizeDisposition::ExistingOutcome`. The engine never lies by rolling the
upload back to Ready and blindly repeating an external effect.

## Current-document resume

An in-memory manager may resume an interrupted transfer in the same current
document when it still owns the original `File`, handle, grant, revision, and
next chunk position. `resumeUpload` is the public ESM helper for that already
configured manager. File identity is checked by name, size, type, and
`lastModified`; the browser never resumes different bytes under an old handle.

After a reload or when the secret grant is no longer retained, the application
must explicitly reacquire short-lived authority. `reacquireUpload` calls the
configured `UploadApplicationPort`, checks the returned file identity, grant,
revision, byte position, chunk position, and resumable state, and then hands the
result to the manager. The corresponding endpoint is an authenticated
application route outside `/__live/`; the reference host demonstrates
`/example/uploads/:handle/reacquire`. Reacquisition reauthorizes the current
principal, session, tenant, component, field, and upload state. It is never an
anonymous status or grant-recovery endpoint.

Pending state is surfaced with stable status text, `aria-busy`, and progressbar
values. Errors are connected with `aria-errormessage`; cancel, retry, and remove
remain labeled buttons and preserve focus across morphs. Reduced motion affects
presentation only. Offline state pauses new transfer work, and reconnect does
not imply that a lost external effect is safe to repeat.

## Cleanup

Cancel is a conditional idempotent lifecycle transition. It aborts active
browser work, asks the server to retire temporary provider work, releases
permits and retained chunks, clears the secret grant, and proposes removal of
the handle from component state. Replacement follows the declared field policy;
the checked example retires the previous temporary upload.

Expiration is independent of grant expiry. The reference profile sets a 24-hour
maximum temporary age. A bounded cleanup worker leases batches of at most 256
records, applies revision-fenced terminal transitions, removes validation
evidence, and idempotently deletes quarantine objects. Lease identity and
duration are bounded; cleanup retries use bounded backoff. A cancellation,
provider failure, or process shutdown must drain owned files, operations,
timers, permits, and queue entries. `CleanupTimedOut` preserves the unfinished
obligation for later cleanup rather than dropping it.

Finalized application data follows host retention policy, not temporary-upload
cleanup. The engine can clean the temporary quarantine object and authority
record only after durable commit/reconciliation makes that safe. Metrics expose
bounded counts and lifecycle/error categories. Browser resource observers key
per-transfer buffer accounting with bounded document-local numeric slots, not
handles; the slots are stable only for the local entry lifetime and carry no
authority. Metrics and observers never expose handles, client filenames,
checksums, grants, provider URLs, or uploaded bytes.
