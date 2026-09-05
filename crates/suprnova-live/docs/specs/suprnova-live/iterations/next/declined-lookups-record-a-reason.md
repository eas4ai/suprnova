# Declined lookups should record why they declined -- staged for next iteration

Status: Staged (not in current contract)
Captured: 2026-09-04
Target domain: `16-cache-variance-privacy-and-stitching.md`

## What it is

RenderCache's lookup telemetry closes its `outcome` attribute over eight
values, one of which is `declined`. Every decline is counted under that one
value and nothing else, so a decline is indistinguishable from any other
decline once it reaches an operator. Three fail-closed limits now produce
declines that no request-visible signal explains: a render that resolves an
anonymous identity through the session fallback, a route keyed only by
`Tenant` whose gate check is treated as per-principal, and an Inertia
document that reads the negotiated locale on a route which declares no
`Locale` variance. Each is correct behaviour. Each is also silent: the route
serves normally, nothing fails, and the cache simply never fills, which is
the failure mode hardest to notice and hardest to attribute.

A future revision may attach a bounded `reason` attribute to the `declined`
outcome, drawn from the classification reasons the engine already
enumerates (`PrincipalObserved`, `TenantObserved`, `SessionValueRead`,
`AuthorizationRead`, `SecretContextRead`, `UndeclaredContext`), the
eligibility decline reasons (`PolicyUncacheable`, `Method`, `Status`,
`Streaming`, `SetsCookie`, `UnsafeHeader`), and the middleware's own guard
branches. The attribute stays a closed, low-cardinality enumeration: it
names which contract refused, never a key, a route parameter, a header
value, or any identity.

## Acceptance criteria

- The `declined` lookup outcome carries a `reason` attribute whose values
  are a closed enumeration fixed at compile time, with no request-derived
  text in it.
- Every branch that declines a store or a serve maps to exactly one reason,
  and adding a decline branch without a reason fails to compile rather than
  falling back to an unattributed default.
- The three named silent limits above are each distinguishable from one
  another and from an ordinary ineligible response in the recorded reason.
- Telemetry's own closed-label contract still holds: the reason set is
  bounded, documented alongside `outcome`, and asserted by the operations
  suite rather than only described in prose.
