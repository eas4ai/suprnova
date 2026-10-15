# Live host adapter and endpoint contract

The kernel accepts normalized typed facts from a framework host. It never reads
raw cookies, CSRF tokens, forwarding headers, credentials, or route internals,
and it cannot truthfully manufacture those facts from browser protocol bytes.

## Trusted request context

The host first creates a non-authoritative `LiveRequestContextCandidate` from
the current route and slot, selected registered mount, normalized scope facts,
all required check facts, opaque capabilities, and an expiry. The production
`LiveRequestContextValidator` issues `TrustedLiveRequestContext` only after it
proves:

- route, slot, component contract, schemas, and selected protocol match the
  immutable mount catalog and component registry;
- scope, session, principal, and tenant fingerprints agree with the bound host
  capabilities and mount requirements;
- origin, CSRF, session, principal, tenant, proxy, rate-limit, and route
  middleware checks are all present and current; and
- every check is either `Passed` or has the one typed `NotRequired` policy
  reason valid for that check.

The context lifetime is bounded and clipped to the earliest check expiry. Its
normal `Debug` output is redacted. There is no unchecked disposition, public
zero-input verified constructor, raw credential field, or reusable domain
authorization decision.

## Host adapter contract

Iteration 005's Suprnova adapter must own truthful normalization and enforcement
for HTTP method/media/cache admission, origin, CSRF, session rotation, current
principal and tenant, trusted proxy facts, rate limiting, route middleware, the
route/slot mount catalog, and request-scoped application capabilities. It must
also own concrete session, authorization, validation, transaction, outbox,
clock, randomness, ledger, and application-service providers. That production
adapter is not claimed complete until its framework integration tests pass.

The adapter passes complete bounded body bytes and a validated context into the
host-neutral endpoint, then translates `LiveEndpointResponse` into Suprnova's
real response type. It must not reconstruct a context from a snapshot, reuse one
across requests or identity changes, skip a declared check, or treat a
conformance provider as production authentication or authorization.

The types in the integrated internal crate define and test that adapter
contract. No router, middleware, request/response, session, auth, tenant, or
transaction adapter is claimed complete merely because those types now share
the Suprnova workspace.

## Endpoint service

`LiveEndpointService` accepts only `POST`, an explicit cache-bypass decision,
complete bounded bytes, and one of these exact media types:

```text
application/vnd.suprnova.live+json; charset=utf-8; version=1
application/vnd.suprnova.live+json; charset=utf-8; version=2
```

Before kernel dispatch it checks request size, current context, whole protocol
version, context/catalog/registry identity, signed seed or instance authority,
scope binding, and base revision. After dispatch it rechecks clock monotonicity
and context expiry, parses and validates the complete versioned response,
verifies correlation/outcome class and any successor snapshot, canonically
re-encodes it, and enforces the final response limit.

The endpoint owns status, headers, cache policy, cookies, and media type. Every
response is `no-store`, `nosniff`, `no-referrer`, carries a restrictive
`default-src 'none'; frame-ancestors 'none'` CSP and exact content length, and
includes the Live media type only for a validated protocol body. A method error
adds `Allow: POST`. Component islands cannot inject any of those fields.

Closed kernel-to-HTTP mapping is:

| Kernel outcome | HTTP status |
| --- | --- |
| accepted or retained duplicate | `200 OK` |
| validation/policy rejected | `422 Unprocessable Entity` |
| concealed authentication/authorization failure | `404 Not Found` with no protocol body |
| conflict or refresh-required | `409 Conflict` |
| fatal | `500 Internal Server Error` |

Normalization uses `405`, `415`, `413`, `400`, `404`, `409`, or `500` according
to its closed failure kind. No partial or unvalidated kernel body becomes HTTP
success.

## Security boundary

A valid HMAC proves signed integrity and purpose, not request authenticity,
secrecy, current authorization, tenant membership, or ledger authority. The
trusted context and verified snapshot are distinct capabilities and both are
required. Browser-controlled component/action/model strings resolve only
through the trusted catalog and immutable generated registry.

Host checks fail before component dispatch. Current authorization runs again
inside action execution after hydration. Signed child parameters have their own
purpose key and cannot be substituted with a snapshot or raw JSON. Request,
response, context, provider, and endpoint errors retain closed categories and
byte counts, never credentials, snapshots, state, proposals, or arguments.

The real action route uses its existing middleware/context/attestation path for
protocol-v2 `params_changed`. Its `child_parameters` value is an exact-key
carrier containing a v2 envelope and signed accepted parent snapshot. Endpoint
admission verifies that carrier, the independently supplied current child
snapshot, exact route/slot/scope/session/tenant/component/instance bindings,
and non-batched operation shape before protected component or host transaction
work. Parent route/slot/component claims only select a candidate in the existing
immutable catalog; the matched registration supplies trusted build, route,
slot, contract, schemas, and current request scope for parent snapshot
verification. Ledger eligibility distinguishes concealed logical authority
failure from provider unavailability. Accepted response projection returns the
engine's sealed bytes unchanged with the normal cache and content headers.

## Failure mapping

Wrong method/media/charset/version, cache attempts, missing or expired context,
oversized bytes, malformed batches, catalog drift, scope/revision mismatch, and
invalid signatures fail before application execution. Changed authorization is
concealed under host policy. Duplicate acceptance with no retained body maps to
refresh-required; it does not replay work.

Kernel unavailability, invalid or oversized kernel output, clock failure, and
unsafe successor state return an empty `500` response intent. Context expiry
after kernel completion suppresses publication. Action/transaction/ledger
failures use the protocol recovery rules documented in
[actions and validation](actions-and-validation.md) and
[protocol v2](protocol-v2.md).
