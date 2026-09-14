# Live session revocation

Slug: live-session-revocation
Requirements: LIVE-019, LIVE-020
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

## Decisions to record

- Revocation substrate: an in-process revocation of memberships keyed by
  session fingerprint and principal, driven from the session lifecycle
  (invalidate, regenerate, `destroy_for_user`), plus a store-backed
  re-verification per membership on a bounded interval for the other
  nodes of a deployment. The alternative, a store read on every delivery
  for every membership, costs one store round trip per membership per
  event and is refused.
- Re-verification interval: ten seconds per membership. The alternative,
  re-verifying only at subscription renewal, bounds the cross-node leak at
  the subscription lifetime (120 s) instead.

## Deliverables

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

- Requirements: `docs/spec/live.md` (LIVE-019, LIVE-020).
- Mechanisms: `.cairn/mechanisms/live-session-revocation`,
  `.cairn/mechanisms/live-session-reverification`.
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
