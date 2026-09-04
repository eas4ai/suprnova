# Session identity reads versus session content reads -- staged for next iteration

Status: Staged (not in current contract)
Captured: 2026-09-04
Target domain: `16-cache-variance-privacy-and-stitching.md`

## What it is

`Auth::id()` resolves identity through request-scoped state first and falls
back to `session()` for an anonymous visitor. `session()` always records a
session read, and RenderCache's classification narrows any session read
straight to `Uncacheable`, with no exception for a read that only resolves
who is signed in. The consequence reaches further than the fallback itself:
an anonymous visitor of a route whose render calls `Auth::id()` never
caches at all, and a session-authenticated render is `Uncacheable` rather
than `PrivateCached`, even on a route that declares `Principal` variance
correctly and would otherwise cache one representation per signed-in
visitor.

A future revision may distinguish a session read that resolves identity
only from a session read that touches actual session content. The former
would record principal material and be compared by value the way every
other identity read already is; the latter would keep forcing
`Uncacheable` exactly as it does today. This widens what can cache and
needs its own adversarial review before it ships: today's blanket rule is
simple to state and impossible to get subtly wrong, and any replacement
has to preserve that a session read that resolves anything beyond identity,
or that cannot be proven to resolve identity alone, still narrows to
`Uncacheable`.

## Acceptance criteria

- A session read that only ever resolves identity is comparable by value
  against a route's declared `Principal` material, the same way a request-
  state identity resolution already is.
- A session read that touches any other session field, or that cannot be
  statically distinguished from one that might, still forces
  `Uncacheable` unconditionally.
- The existing proof that an anonymous session-resolved render stays
  `Uncacheable`
  (`an_anonymous_render_that_resolves_identity_through_the_session_stays_uncacheable`)
  is replaced by, or joined by, evidence of the corrected behavior; it is
  not silently invalidated by a change that makes its own assertion false.
- The privacy leak suite gains a case proving a per-user session-backed
  guard whose only observed effect on rendering is identity resolution
  caches correctly under `Principal` variance without leaking another
  visitor's page.
