# Session identity reads versus session content reads -- delivered in iteration 005

Status: Delivered (in iteration 005, 2026-09-07)
Captured: 2026-09-04
Delivered: 2026-09-07
Target domain: `16-cache-variance-privacy-and-stitching.md`

Delivered by commit `3e753483`, under ruling R24 of the RenderCache budget
harness and qualification plan, with the closed field set typed at
`a862740c`. `Auth::id()`'s fallback now reads only the session's
authentication identifiers - the default guard's user id and a named guard's
own id, a set closed by a private enum - and records a principal read plus,
when there is an id, the principal value. Every other session value still
records a session read and still narrows to `Uncacheable`. The four
acceptance criteria below are met; the note is kept as the record of why the
change was made and what it was required to preserve, not as an open item.

## What it is

The problem as captured on 2026-09-04, in the present tense of that date:

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

Each one is followed by the evidence that met it. Every named test is in
`framework/tests/render_cache/middleware.rs`.

- A session read that only ever resolves identity is comparable by value
  against a route's declared `Principal` material, the same way a request-
  state identity resolution already is. Met: `session_identity` records
  `observe_principal_read` and `observe_principal_value`, and
  `a_session_resolved_principal_is_stored_and_partitioned_per_principal`
  proves the value comparison holds.
- A session read that touches any other session field, or that cannot be
  statically distinguished from one that might, still forces
  `Uncacheable` unconditionally. Met: the accessor takes a closed private
  enum of the two permitted fields, so no other read can reach it, and
  `session_mut_reads_are_observed_and_force_uncacheable` still holds.
- The proof that an anonymous session-resolved render stays `Uncacheable` is
  replaced by, or joined by, evidence of the corrected behavior; it is not
  silently invalidated by a change that makes its own assertion false. It was
  replaced, by
  `an_anonymous_render_resolving_identity_through_the_session_caches_anonymously`,
  which pins the anonymous key, the signed-in visitor's separate private key,
  and that the two never meet.
- The privacy leak suite gains a case proving a per-user session-backed
  guard whose only observed effect on rendering is identity resolution
  caches correctly under `Principal` variance without leaking another
  visitor's page. Met, in the middleware suite rather than the leak suite:
  `a_session_resolved_principal_is_stored_and_partitioned_per_principal`
  proves the partitioning,
  `a_session_resolved_principal_render_observes_the_row_it_was_resolved_from`
  proves a profile write invalidates the entry, and
  `a_session_resolved_principal_is_declined_where_no_principal_variance_is_declared`
  proves the same read on an undeclared route is refused.
