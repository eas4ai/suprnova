# Live and RenderCache hardening

Slug: live-and-render-cache-hardening
Requirements: CACHE-003, CACHE-004, CACHE-001, CACHE-002, CACHE-009, CACHE-008, LIVE-016, LIVE-017, CACHE-006, CACHE-005, LIVE-018, CACHE-010, CACHE-007
Rests on: LIVE-001, LIVE-002, LIVE-005, LIVE-006

## Goal

Every defect the 2026-09-13 adversarial audit reproduced is closed by a
regression test that fails on the current tree and passes after the fix,
in the audit's remediation order: browser execution and byte replay
first, then invalidation and database routing, then the Live
authorization and transaction contracts, then the rest.

Drafted 2026-09-13 from the audit report (Astra, thirteen findings, eight
High and five Medium; ten reproduced over loopback HTTP, three confirmed
from source). The owner ruled at 11:13 that remediation precedes the
component library. Not Agreed until the developer confirms the thirteen
requirements and falsifiers as one set.

## Decisions to record before code

- CACHE-004, nonce handling (Consequential): decline complete-entry
  storage for nonce-bearing responses, or generalize the stitched
  template mechanism to re-nonce every hit. The audit recommends either;
  the developer rules.
- CACHE-009, write atomicity (Consequential): one transaction for data
  and generation on the autocommit path, or a transactional outbox with
  fail-closed serving while invalidation is uncertain.
- LIVE-017, action transactions (Consequential): implement the ambient
  transaction, or refuse registration of Required actions until it
  exists. The audit recommends refusing until real; the connection-pool
  deadlock history in `framework/src/live/ports/transaction.rs` is the
  reason it was left a no-op.

## Deliverables

- `framework/tests/render_cache/hardening.rs`: the audit's cache and
  request-directive probes ported as named tests, one per CACHE
  requirement, asserting the corrected behavior.
- `framework/tests/live/hardening.rs`: the revocation, transaction, and
  concurrent-issuance probes, one per LIVE-016 to LIVE-018.
- The fixes in `framework/src/render_cache/`,
  `crates/suprnova-live/src/render_cache/`,
  `framework/src/live/async_updates.rs`,
  `framework/src/live/ports/transaction.rs`, and the registry, each with
  its regression test passing.
- Dated "Decisions and revisions" entries in Live specs 14, 15, 16, 17,
  18 where behavior is clarified (LIVE-014).
- `CHANGELOG.md` Security entries under the current version's section
  and the six locale mirrors, re-stamped.

## Records

- Requirements: `docs/spec/render-cache.md` (CACHE), `docs/spec/live.md`
  (LIVE-016 to LIVE-018).
- Mechanisms: one per requirement under `.cairn/mechanisms/cache-*` and
  `.cairn/mechanisms/live-*`, each a nextest filter on its test.
- Evidence: `.cairn/evidence/`, committed after each `cairn check`.
- Review: `.cairn/reviews/live-and-render-cache-hardening.md`.

## Tests that prove it

Each mechanism is the ported probe with its assertion inverted to the
corrected behavior; the safe violating example is the probe as Astra
wrote it, which passes against the current tree and must fail after the
fix. Both directions are recorded in the review.

## Done when

Every mechanism above passes on a committed tree, the review is recorded,
and `cairn wake` says Done.
