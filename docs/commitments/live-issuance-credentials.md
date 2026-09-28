# Live issuance credentials

Slug: live-issuance-credentials
Requirements: LIVE-022, LIVE-023
Rests on: LIVE-018, LIVE-001, LIVE-002

## Goal

Every issuance the per-scope limit admits yields a subscription the
browser can connect with. Today two concurrent issuances of one scope in
the same millisecond mint the same signed descriptor, and the host's
credential store, keyed by that descriptor's binding, keeps only the
last secret: the earlier request's connect is refused with
`async_authority_invalid`. The LIVE-018 probe saw 39 of 512 admitted
requests fail this way.

Drafted 2026-09-14 from the backlog entry
`concurrent-sse-issuance-on-one-document-instance-answers-some-requests-403-async-authority-invalid`
after the developer named it the next commitment at 09:05.

Widened 2026-09-14 by the developer's answer on escalation `live-022`
(10:06, `ok`): the LIVE-022 baseline also showed one admitted issuance in
512 answering 503 `async_unavailable`, because the claims an issuance
publishes while its envelope context is constructed live in one shared
slot that a concurrent issuance overwrites. That is LIVE-023, checked by
the same probe.

## Decisions recorded

Agreed by the developer at 09:05 on 2026-09-14 ("confirmed"), taking the
recommended option; recorded in
`docs/decisions/identical-descriptors-from-concurrent-issuance-are-told-apart-in-the-host-credential-store-not-by-a-nonce-in-the-claims.md`:

- Where identical descriptors are told apart: in the host's credential
  store, which keeps every secret issued for a binding until each is
  consumed or expires, so identical descriptors from concurrent
  issuances each connect once with their own secret. The alternative,
  a per-issuance nonce inside the signed claims, makes every descriptor
  unique but changes the descriptor schema, the browser's generated
  contract, and the conformance fixture for a collision only concurrent
  issuance in one millisecond produces.

## Deliverables

- `framework/tests/live/hardening.rs`:
  `concurrent_issuance_keeps_every_credential` (LIVE-022), the LIVE-018
  probe asserting that every admitted request answers 201, failing on the
  current tree.
- `framework/src/live/ports/subscription.rs`: the credential store holds
  every unconsumed secret per binding, bounded by the existing entry cap.
- `framework/src/live/async_updates.rs`: the claims under construction
  are keyed by subscription id instead of held in one slot (LIVE-023).
- The backlog entry closed with the realizing commit named.
- Dated "Decisions and revisions" entry in Live spec 14.
- `CHANGELOG.md` Fixed entry under the current version's section and the
  six locale mirrors, re-stamped.

## Records

- Requirements: `docs/spec/live.md` (LIVE-022, LIVE-023).
- Mechanism: `.cairn/mechanisms/live-issuance-credentials`.
- Evidence: `.cairn/evidence/`, committed after each `cairn check`.
- Review: `.cairn/reviews/live-issuance-credentials.md`.

## Tests that prove it

The mechanism is the LIVE-018 probe with its histogram turned into the
assertion: every request the limit admits answers 201. The safe violating
example is the probe as it stands today, whose histogram shows the 403s;
both directions are recorded in the review.

## Done when

The mechanism passes on a committed tree, the review is recorded, and
`cairn wake` says Done.
