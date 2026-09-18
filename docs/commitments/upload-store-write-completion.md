# A store write completes when the bytes have landed

Slug: upload-store-write-completion
Requirements: LIVE-038
Rests on: LIVE-010

## Goal

The Live gate's result reflects the upload provider's behavior and not the
buffering of the store it runs against: a write the provider awaits has
reached the object when it completes, so the read that follows sees those
bytes. Promoted from the backlog on the developer's ruling of 2026-09-17,
answering escalation `live-010` with instead; the decision is
`docs/decisions/the-quarantine-store-the-live-gate-runs-completes-a-write-only-when-the-bytes-have-reached-the-file.md`.

## Deliverables

- `write_all_fragmented` flushes the tokio file before the operation
  completes, and says in its comment why completion has to mean the bytes
  reached the file.
- The mechanism runs the whole upload provider file sixty times, labeling
  each run, and fails on the first red one.

## Records

- Requirement: `docs/spec/live.md` (LIVE-038).
- Mechanism: `.cairn/mechanisms/live-upload-store-flush`.
- Evidence: `.cairn/evidence/`, committed after each `cairn check`.
- Review: `.cairn/reviews/upload-store-write-completion.md`.

## Done when

The mechanism passes on a committed tree, the Live gate and the repository
gate pass, the review is recorded, and `cairn wake` says Done.
