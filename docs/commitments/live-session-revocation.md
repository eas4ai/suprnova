# Live session revocation

Slug: live-session-revocation
Requirements: LIVE-019, LIVE-020, LIVE-021
Rests on: LIVE-016, LIVE-001, LIVE-002

## Goal

An async Live membership never outlives the session that opened it. The
shipped LIVE-016 fix re-asks the stream's Gate before each delivery; this
commitment closes the clauses of LIVE-016 that had no host substrate: a
membership whose session was destroyed, on this node or another, stops
receiving events and is retired.

Drafted 2026-09-13 from the developer's `ok` on escalation `live-016`
at 21:21: build session binding as one follow-up commitment before the
component library resumes.

## Decisions recorded

Agreed by the developer at 23:22 on 2026-09-13 ("I will accept your
recommendations"), taking the recommended option in each case:

- Revocation substrate: session invalidation, regeneration, and
  `destroy_for_user` revoke this node's memberships in process, keyed by
  session fingerprint and user id; other nodes learn through a bounded
  store re-check before delivery
  (`docs/decisions/session-revocation-reaches-memberships-in-process-with-a-store-re-check-for-other-nodes.md`).
- Re-verification interval: ten seconds per membership
  (`docs/decisions/a-membership-re-verifies-its-session-against-the-store-at-most-every-ten-seconds.md`).

Reopened 2026-09-14 08:44 for LIVE-021 after the developer's `ok` on
escalation `live-019`: the review found that plain `Auth::logout` keeps
the session row, so the scaffold's logout left streams delivering.

## Deliverables

- `framework/tests/live/hardening.rs`: `plain_logout_ends_delivery`
  (LIVE-021), failing on the tree of `cf7332d1`; the default guard's
  logout retires the memberships its session opened for that user.

- `framework/tests/live/hardening.rs`: `revoked_session_ends_delivery`
  (LIVE-019) and `stale_store_session_ends_delivery` (LIVE-020), each
  failing on the current tree.
- `IssuedRecord` carries the session fingerprint and the session id at
  issuance; `AsyncState` gains `revoke_session` and `revoke_principal`,
  called from the session middleware's regeneration-aware persistence
  step, `invalidate_session`, and the store's `destroy_for_user` path;
  `publish` re-verifies each candidate's session against the store when
  its last verification is older than the interval.
- Dated "Decisions and revisions" entries in Live spec 14, closing the
  2026-09-13 note that the session and revocation-state clauses had no
  host substrate.
- `CHANGELOG.md` Security entry under the current version's section and
  the six locale mirrors, re-stamped.

## Records

- Requirements: `docs/spec/live.md` (LIVE-019 to LIVE-021).
- Mechanisms: `.cairn/mechanisms/live-session-revocation`,
  `.cairn/mechanisms/live-session-reverification`,
  `.cairn/mechanisms/live-session-deauthentication`.
- Evidence: `.cairn/evidence/`, committed after each `cairn check`.
- Review: `.cairn/reviews/live-session-revocation.md`.

## Tests that prove it

Each mechanism is a probe in the shape of the hardening tests: subscribe,
destroy the session (through the auth facade for LIVE-019, through the
store for LIVE-020), publish, and assert that the stream carries nothing
and the membership is gone. The safe violating example is the same probe
asserting delivery, which passes on the current tree and must fail after
the fix. Both directions are recorded in the review.

## Done when

Both mechanisms pass on a committed tree, the review is recorded, and
`cairn wake` says Done.
