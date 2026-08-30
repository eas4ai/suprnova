# Iteration 004 Task 7 quality audit

Date: 2026-08-29

## In-iteration correction

The Task 7 documentation review found that the public browser upload-resource
observer and the checked `U4/16` evidence forwarded raw upload handles as
per-transfer accounting keys. That contradicted Iteration 004's existing secret
and observability boundary; it was not a new feature or a scope expansion.

The correction keeps handles inside upload transport and service authority. The
observer and browser/server benchmark evidence now use bounded, non-identifying,
document-local numeric slots that remain stable for the lifetime of the observed
entry. Exact evidence schemas reject a substituted raw `handle` field, and
sentinels reject generated handles and grants in observer JSON and checked
evidence.

The same review corrected two documentation claims without changing shipped
architecture: the built-in WebSocket adapter accepts only `session_cookie`
authorization, while bearer-authorized or cross-origin WebSocket requires an
application-supplied custom transport; and the reference host, Node static host,
direct-provider bridge, fault controls, and benchmarks are conformance-only test
tools rather than production administration, Suprnova integration, or vendor
integration.

## Verification

- The observer and checked-evidence sentinels failed before the correction.
- Raw-handle schema mutations fail for browser chunk, server chunk, and server
  completion evidence.
- The implementation-document checker detects seven semantic mutations,
  including WebSocket bearer inversion and both production-boundary inversions.
- The regenerated checked `U4/16` reference is unqualified and retains
  `qualifiedBaseline: null`.
- `CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 scripts/gate.sh` passed after the
  correction.
