# Authorization reads should record the identity they consulted -- staged for next iteration

Status: Staged (not in current contract)
Captured: 2026-09-04
Target domain: `16-cache-variance-privacy-and-stitching.md`

## What it is

`Gate::allows` records only that a private authorization decision was
evaluated; it never records what the decision actually consulted.
RenderCache's classification therefore maps every `AuthorizationRead`
unconditionally to the `Principal` dimension, and the value guard requires
a route to declare `Principal` before an authorization-checked render can
ever be stored - even on a route keyed only by `Tenant` whose gate check is
genuinely per-tenant rather than per-user. Such a route never caches unless
it also declares `Principal`, which is safe but needlessly narrow: nothing
here can currently tell a per-tenant gate from a per-user one, so treating
every decision as per-user is the only default that does not risk a leak.

A future revision may have `Gate` record the principal or tenant identity
it actually consulted while evaluating a decision, as material comparable
the same way every other identity read already is. The value guard could
then compare an authorization decision's own resolved subject against the
key's declared variance instead of assuming `Principal` unconditionally,
letting a genuinely per-tenant gate cache under `Tenant` alone. The wider
gap this sits inside is the whole class of public, uninstrumented accessors
onto the authenticated identity, of which `Request::auth_user_id` is one
member, and the only coverage that class has today is the single tripwire
`the_uninstrumented_request_accessor_carries_no_identity_and_no_body_crosses`,
which measures that one accessor rather than proving anything about the
class.

## Acceptance criteria

- `Gate::allows` (or its evaluation path) records the identity axis, and
  the concrete identity, a decision actually consulted, not merely that a
  decision ran.
- A decision that consults only a tenant-scoped fact classifies and
  key-compares as `Tenant` material, not `Principal`; a decision that
  consults a per-user fact keeps requiring `Principal` as it does today.
- A decision that consults both axes, or one this recording cannot resolve
  cleanly, keeps the current conservative behavior (requiring `Principal`)
  rather than guessing.
- The privacy leak suite gains a case proving a per-tenant-only gate check
  does not leak one tenant's authorized page to a different tenant sharing
  the same key once this widening ships.
